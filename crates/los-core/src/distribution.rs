// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - TOKEN DISTRIBUTION STATE
//
// Tracks remaining public supply available for PoW mining.
// Public allocation: 21,158,413 LOS (total supply minus dev treasury).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};

/// Maximum public supply cap: 21,158,413 LOS in CIL
pub const PUBLIC_SUPPLY_CAP: u128 = 21_158_413 * crate::CIL_PER_LOS;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DistributionState {
    pub remaining_supply: u128,
}

impl Default for DistributionState {
    fn default() -> Self {
        Self::new()
    }
}

impl DistributionState {
    pub fn new() -> Self {
        Self {
            remaining_supply: PUBLIC_SUPPLY_CAP,
        }
    }
}
