#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <objc/message.h>
#import <objc/runtime.h>
#import <dispatch/dispatch.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

static id g_updater_controller = nil;
static bool g_updater_ready = false;
static char g_last_error[1024] = {0};
static char g_feed_url[2048] = {0};
static char g_last_check_utc[128] = {0};

static void set_last_error(NSString *message) {
    if (message == nil) {
        g_last_error[0] = '\0';
        return;
    }

    const char *utf8 = [message UTF8String];
    if (utf8 == NULL) {
        g_last_error[0] = '\0';
        return;
    }

    snprintf(g_last_error, sizeof(g_last_error), "%s", utf8);
}

static void write_utf8_to_buffer(NSString *value, char *buffer, size_t buffer_len) {
    if (buffer == NULL || buffer_len == 0) {
        return;
    }
    buffer[0] = '\0';
    if (value == nil) {
        return;
    }
    const char *utf8 = [value UTF8String];
    if (utf8 == NULL) {
        return;
    }
    snprintf(buffer, buffer_len, "%s", utf8);
}

static NSString *bundle_info_string(NSString *key) {
    NSBundle *bundle = [NSBundle mainBundle];
    id value = [bundle objectForInfoDictionaryKey:key];
    if (value == nil || ![value isKindOfClass:[NSString class]]) {
        return nil;
    }
    NSString *raw = (NSString *)value;
    if ([raw length] == 0) {
        return nil;
    }
    return raw;
}

static bool bundle_info_bool(NSString *key, bool *value_out) {
    if (value_out != NULL) {
        *value_out = false;
    }

    NSBundle *bundle = [NSBundle mainBundle];
    id value = [bundle objectForInfoDictionaryKey:key];
    if (value == nil) {
        return false;
    }

    if ([value isKindOfClass:[NSNumber class]]) {
        if (value_out != NULL) {
            *value_out = [((NSNumber *)value) boolValue];
        }
        return true;
    }

    if ([value isKindOfClass:[NSString class]]) {
        NSString *text = [((NSString *)value) lowercaseString];
        bool interpreted = [text isEqualToString:@"true"] || [text isEqualToString:@"1"] ||
                           [text isEqualToString:@"yes"];
        if (value_out != NULL) {
            *value_out = interpreted;
        }
        return true;
    }

    return false;
}

static NSString *format_utc_timestamp(NSDate *date) {
    if (date == nil) {
        return nil;
    }

    static NSDateFormatter *formatter = nil;
    static dispatch_once_t once_token;
    dispatch_once(&once_token, ^{
      formatter = [[NSDateFormatter alloc] init];
      formatter.locale = [[NSLocale alloc] initWithLocaleIdentifier:@"en_US_POSIX"];
      formatter.timeZone = [NSTimeZone timeZoneWithAbbreviation:@"UTC"];
      formatter.dateFormat = @"yyyy-MM-dd HH:mm:ss 'UTC'";
    });

    return [formatter stringFromDate:date];
}

static NSString *framework_path(void) {
    NSBundle *bundle = [NSBundle mainBundle];
    NSString *frameworks = [bundle privateFrameworksPath];
    if (frameworks == nil || [frameworks length] == 0) {
        return nil;
    }
    return [frameworks stringByAppendingPathComponent:@"Sparkle.framework"];
}

static bool ensure_loaded(void) {
    NSString *path = framework_path();
    if (path == nil) {
        set_last_error(@"App private frameworks directory not found.");
        return false;
    }

    NSBundle *sparkle_bundle = [NSBundle bundleWithPath:path];
    if (sparkle_bundle == nil) {
        set_last_error(@"Sparkle.framework bundle not found in app.");
        return false;
    }

    NSError *error = nil;
    if (![sparkle_bundle isLoaded] && ![sparkle_bundle loadAndReturnError:&error]) {
        NSString *msg = [NSString stringWithFormat:@"Failed to load Sparkle.framework: %@",
                                                   error.localizedDescription ?: @"unknown error"];
        set_last_error(msg);
        return false;
    }

    return true;
}

static id fetch_updater_instance(bool set_error_on_failure) {
    if (!g_updater_ready || g_updater_controller == nil) {
        if (set_error_on_failure) {
            set_last_error(@"Sparkle updater controller is not initialized.");
        }
        return nil;
    }

    SEL updater_sel = sel_registerName("updater");
    if (![g_updater_controller respondsToSelector:updater_sel]) {
        if (set_error_on_failure) {
            set_last_error(@"Sparkle API mismatch: missing updater accessor.");
        }
        return nil;
    }

    id updater = ((id(*)(id, SEL))objc_msgSend)(g_updater_controller, updater_sel);
    if (updater == nil && set_error_on_failure) {
        set_last_error(@"Sparkle updater instance is not available.");
    }
    return updater;
}

static bool resolve_check_target(id *target_out, SEL *selector_out, bool *passes_sender_out) {
    if (target_out != NULL) {
        *target_out = nil;
    }
    if (selector_out != NULL) {
        *selector_out = NULL;
    }
    if (passes_sender_out != NULL) {
        *passes_sender_out = false;
    }

    if (!g_updater_ready || g_updater_controller == nil) {
        set_last_error(@"Sparkle updater controller is not initialized.");
        return false;
    }

    // Preferred path for Sparkle 2: invoke action directly on SPUStandardUpdaterController.
    SEL controller_check_sel = sel_registerName("checkForUpdates:");
    if ([g_updater_controller respondsToSelector:controller_check_sel]) {
        if (target_out != NULL) {
            *target_out = g_updater_controller;
        }
        if (selector_out != NULL) {
            *selector_out = controller_check_sel;
        }
        if (passes_sender_out != NULL) {
            *passes_sender_out = true;
        }
        return true;
    }

    id updater = fetch_updater_instance(true);
    if (updater == nil) {
        return false;
    }

    // Older/alternate path: invoke method on updater directly.
    SEL updater_check_sel = sel_registerName("checkForUpdates");
    if ([updater respondsToSelector:updater_check_sel]) {
        if (target_out != NULL) {
            *target_out = updater;
        }
        if (selector_out != NULL) {
            *selector_out = updater_check_sel;
        }
        return true;
    }

    // Some builds may still expose a sender-based variant.
    SEL updater_check_sender_sel = sel_registerName("checkForUpdates:");
    if ([updater respondsToSelector:updater_check_sender_sel]) {
        if (target_out != NULL) {
            *target_out = updater;
        }
        if (selector_out != NULL) {
            *selector_out = updater_check_sender_sel;
        }
        if (passes_sender_out != NULL) {
            *passes_sender_out = true;
        }
        return true;
    }

    set_last_error(@"Sparkle API mismatch: missing checkForUpdates selector.");
    return false;
}

static bool updater_can_check_now(bool set_error_on_failure) {
    id updater = fetch_updater_instance(set_error_on_failure);
    if (updater == nil) {
        return false;
    }

    SEL can_check_sel = sel_registerName("canCheckForUpdates");
    if (![updater respondsToSelector:can_check_sel]) {
        // If unavailable, do not fail readiness checks on this optional API.
        return true;
    }

    BOOL can_check = ((BOOL(*)(id, SEL))objc_msgSend)(updater, can_check_sel);
    if (!can_check && set_error_on_failure) {
        set_last_error(@"Sparkle updater is busy. Try again in a moment.");
    }
    return can_check;
}

bool loopbox_updater_init(void) {
    @autoreleasepool {
        if (g_updater_ready && g_updater_controller != nil) {
            return true;
        }

        if (!ensure_loaded()) {
            return false;
        }

        Class controller_class = NSClassFromString(@"SPUStandardUpdaterController");
        if (controller_class == Nil) {
            set_last_error(@"SPUStandardUpdaterController class not available.");
            return false;
        }

        SEL init_sel = sel_registerName("initWithStartingUpdater:updaterDelegate:userDriverDelegate:");
        if (![controller_class instancesRespondToSelector:init_sel]) {
            set_last_error(@"Sparkle API mismatch: missing updater controller initializer.");
            return false;
        }

        id allocated = ((id(*)(id, SEL))objc_msgSend)((id)controller_class, sel_registerName("alloc"));
        id controller = ((id(*)(id, SEL, BOOL, id, id))objc_msgSend)(allocated, init_sel, YES, nil, nil);
        if (controller == nil) {
            set_last_error(@"Failed to initialize Sparkle updater controller.");
            return false;
        }

        g_updater_controller = controller;
        g_updater_ready = true;
        set_last_error(nil);
        return true;
    }
}

bool loopbox_updater_can_check(void) {
    @autoreleasepool {
        if (!g_updater_ready && !loopbox_updater_init()) {
            return false;
        }

        if (!resolve_check_target(NULL, NULL, NULL)) {
            return false;
        }

        set_last_error(nil);
        return true;
    }
}

bool loopbox_updater_check_for_updates(void) {
    @autoreleasepool {
        if (!g_updater_ready && !loopbox_updater_init()) {
            return false;
        }

        if (!loopbox_updater_can_check()) {
            return false;
        }

        if (!updater_can_check_now(true)) {
            return false;
        }

        id target = nil;
        SEL check_sel = NULL;
        bool passes_sender = false;
        if (!resolve_check_target(&target, &check_sel, &passes_sender) || target == nil ||
            check_sel == NULL) {
            return false;
        }

        if (passes_sender) {
            ((void(*)(id, SEL, id))objc_msgSend)(target, check_sel, nil);
        } else {
            ((void(*)(id, SEL))objc_msgSend)(target, check_sel);
        }
        set_last_error(nil);
        return true;
    }
}

const char *loopbox_updater_last_error(void) {
    return g_last_error;
}

const char *loopbox_updater_feed_url(void) {
    @autoreleasepool {
        NSString *feed = bundle_info_string(@"SUFeedURL");
        write_utf8_to_buffer(feed, g_feed_url, sizeof(g_feed_url));
        return g_feed_url;
    }
}

bool loopbox_updater_automatic_checks_enabled(bool *value_out) {
    @autoreleasepool {
        bool raw_value = false;
        bool has_value = bundle_info_bool(@"SUEnableAutomaticChecks", &raw_value);
        if (value_out != NULL) {
            *value_out = raw_value;
        }
        return has_value;
    }
}

const char *loopbox_updater_last_check_utc(void) {
    @autoreleasepool {
        g_last_check_utc[0] = '\0';

        if (!g_updater_ready && !loopbox_updater_init()) {
            return g_last_check_utc;
        }

        id updater = fetch_updater_instance(false);
        if (updater == nil) {
            return g_last_check_utc;
        }

        SEL last_check_sel = sel_registerName("lastUpdateCheckDate");
        if (![updater respondsToSelector:last_check_sel]) {
            return g_last_check_utc;
        }

        id raw_date = ((id(*)(id, SEL))objc_msgSend)(updater, last_check_sel);
        if (raw_date == nil || ![raw_date isKindOfClass:[NSDate class]]) {
            return g_last_check_utc;
        }

        NSString *formatted = format_utc_timestamp((NSDate *)raw_date);
        write_utf8_to_buffer(formatted, g_last_check_utc, sizeof(g_last_check_utc));
        return g_last_check_utc;
    }
}
