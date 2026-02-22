// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - TOKEN DISTRIBUTION STATE
//
// Tracks remaining public supply and total burned value.
// Public allocation: 21,158,413 LOS (total supply minus dev treasury).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::CIL_PER_LOS;
use serde::{Deserialize, Serialize};

/// Maximum public supply cap: 21,158,413 LOS in CIL
pub const PUBLIC_SUPPLY_CAP: u128 = 21_158_413 * CIL_PER_LOS;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DistributionState {
    pub remaining_supply: u128,
    pub total_burned_usd: u128,
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
            total_burned_usd: 0,
        }
    }

    pub fn calculate_yield(&self, burn_amount_usd: u128) -> u128 {
        if self.remaining_supply == 0 {
            return 0;
        }

        // SECURITY FIX V4#5: Integer math to avoid f64 precision drift
        // Formula: yield = burn_usd * remaining_supply / PUBLIC_SUPPLY_CAP
        // Use checked arithmetic to prevent overflow
        // Scale: burn_amount_usd is in $0.01 units, result is in CIL
        let public_cap_los: u128 = 21_158_413;

        // yield_cil = burn_amount_usd * remaining_supply / (public_cap_los * CIL_PER_LOS)
        // To avoid overflow with large numbers, divide before multiply where possible
        // remaining_supply / PUBLIC_SUPPLY_CAP gives the scarcity multiplier
        // burn_amount_usd * CIL_PER_LOS gives value in CIL

        // Safe path: (burn_usd * remaining) / public_cap
        // With intermediate u128: max burn_usd ~10^12, remaining ~10^22 → ~10^34, fits u128 (max ~10^38)
        //
        // SECURITY FIX C-08: On overflow, use divide-before-multiply fallback
        // instead of returning 0 (which would silently destroy burned funds).
        let denominator = public_cap_los * CIL_PER_LOS;
        match burn_amount_usd.checked_mul(self.remaining_supply) {
            Some(numerator) => numerator / denominator,
            None => {
                // Overflow fallback: split remaining = quotient*denominator + remainder.
                // yield = burn * quotient + burn * remainder / denominator
                //
                // SECURITY FIX L-03: Previous code used (burn / denominator * remainder)
                // which truncates to 0 when burn < denominator, losing the entire
                // remainder contribution. Now we try (burn * remainder) first, which
                // is smaller than the original product and usually fits u128.
                let quotient = self.remaining_supply / denominator;
                let remainder = self.remaining_supply % denominator;
                let main = burn_amount_usd.saturating_mul(quotient);
                let correction = match burn_amount_usd.checked_mul(remainder) {
                    Some(v) => v / denominator,
                    None => {
                        // Second-level fallback: split burn_amount_usd too
                        let burn_q = burn_amount_usd / denominator;
                        let burn_r = burn_amount_usd % denominator;
                        burn_q.saturating_mul(remainder)
                            + burn_r.saturating_mul(remainder) / denominator
                    }
                };
                main.saturating_add(correction)
            }
        }
    }
}
