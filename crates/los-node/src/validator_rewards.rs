// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UNAUTHORITY (LOS) - PER-BLOCK VALIDATOR REWARDS
//
// Distributes transaction fees to validators who finalize blocks.
// Each validator's share = block_fee / validator_count.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Calculate validator reward from a finalized Send block's fee.
/// Validators who participate in consensus for a block share its fee.
///
/// - `block_fee_cil`: The fee (in CIL) attached to the Send block.
/// - `validator_count`: Number of validators who confirmed this block.
///
/// Returns the per-validator reward in CIL.
#[allow(dead_code)]
pub fn calculate_validator_reward(block_fee_cil: u128, validator_count: u32) -> u128 {
    if validator_count == 0 || block_fee_cil == 0 {
        return 0;
    }
    block_fee_cil / (validator_count as u128)
}
