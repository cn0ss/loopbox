use super::license::{current_license_tier, LicenseTier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopboxEdition {
    Free,
    Commercial,
}

pub fn current_edition() -> LoopboxEdition {
    match current_license_tier() {
        LicenseTier::None => LoopboxEdition::Free,
        LicenseTier::Commercial => LoopboxEdition::Commercial,
    }
}

pub fn edition_label() -> &'static str {
    match current_edition() {
        LoopboxEdition::Free => "free",
        LoopboxEdition::Commercial => "commercial",
    }
}
