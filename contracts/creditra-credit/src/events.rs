/// Typed event emitted when an operator waives the grace period on a draw.
///
/// # Fields
///
/// * `credit_line_id` — The credit line the draw belongs to.
/// * `draw_id`        — The draw whose grace period was waived.
/// * `waived_by`      — Address of the operator who performed the waiver.
/// * `block_height`   — Chain height at which the waiver was recorded.
/// * `memo`           — Optional human-readable reason supplied by the operator.
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Timestamp};

/// Structured event emitted on the CosmWasm response when a grace period is
/// waived.  Indexers and off-chain listeners can subscribe to the
/// `"grace_period_waived"` event type to observe all waivers.
#[cw_serde]
pub struct GracePeriodWaivedEvent {
    /// Credit line the draw belongs to.
    pub credit_line_id: u64,
    /// Draw whose grace period was waived.
    pub draw_id: u64,
    /// Operator that authorised the waiver.
    pub waived_by: Addr,
    /// Block timestamp of the waiver.
    pub timestamp: Timestamp,
    /// Chain height at the waiver.
    pub block_height: u64,
    /// Optional human-readable reason.
    pub memo: String,
}

impl GracePeriodWaivedEvent {
    /// Convert into CosmWasm response attributes for the `"grace_period_waived"`
    /// event emitted via [`cosmwasm_std::Response::add_event`].
    pub fn into_attributes(self) -> Vec<cosmwasm_std::Attribute> {
        vec![
            cosmwasm_std::Attribute {
                key: "credit_line_id".to_string(),
                value: self.credit_line_id.to_string(),
            },
            cosmwasm_std::Attribute {
                key: "draw_id".to_string(),
                value: self.draw_id.to_string(),
            },
            cosmwasm_std::Attribute {
                key: "waived_by".to_string(),
                value: self.waived_by.to_string(),
            },
            cosmwasm_std::Attribute {
                key: "block_height".to_string(),
                value: self.block_height.to_string(),
            },
            cosmwasm_std::Attribute {
                key: "memo".to_string(),
                value: self.memo,
            },
        ]
    }
}
