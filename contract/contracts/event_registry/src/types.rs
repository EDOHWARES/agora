use soroban_sdk::{contracttype, Address, Map, String};

/// Represents a ticket tier with its own pricing and supply
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketTier {
    /// Name of the tier (e.g., "General", "VIP", "Reserved")
    pub name: String,
    /// Price for this tier in stroops
    pub price: i128,
    /// Maximum tickets available for this tier
    pub tier_limit: i128,
    /// Current number of tickets sold for this tier
    pub current_sold: i128,
    /// Indicates whether tickets in this tier can be refunded by the buyer
    pub is_refundable: bool,
}

/// Represents information about an event in the registry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventInfo {
    /// Unique identifier for the event
    pub event_id: String,
    /// The wallet address of the event organizer
    pub organizer_address: Address,
    /// The address where payments for this event should be routed
    pub payment_address: Address,
    /// The percentage fee taken by the platform (e.g., 5 for 5%)
    pub platform_fee_percent: u32,
    /// Whether the event is currently active and accepting payments
    pub is_active: bool,
    /// Timestamp when the event was created
    pub created_at: u64,
    /// IPFS Content Identifier storing rich metadata details
    pub metadata_cid: String,
    /// Maximum number of tickets available for this event (0 = unlimited)
    pub max_supply: i128,
    /// Current number of tickets that have been successfully purchased
    pub current_supply: i128,
    /// Map of tier_id to TicketTier for multi-tiered pricing
    pub tiers: Map<String, TicketTier>,
}

/// Payment information for an event
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentInfo {
    /// The address where payments for this event should be routed
    pub payment_address: Address,
    /// The percentage fee taken by the platform
    pub platform_fee_percent: u32,
    /// Map of tier_id to TicketTier for multi-tiered pricing
    pub tiers: Map<String, TicketTier>,
}

/// Multi-signature configuration for admin operations
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiSigConfig {
    /// List of administrator addresses
    pub admins: Vec<Address>,
    /// Number of signatures required to execute a proposal
    pub threshold: u32,
}

/// Types of proposed changes that require multi-sig approval
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalType {
    /// Change the platform wallet address
    SetPlatformWallet(Address),
    /// Add a new administrator
    AddAdmin(Address),
    /// Remove an existing administrator
    RemoveAdmin(Address),
    /// Change the signature threshold
    SetThreshold(u32),
}

/// Represents a proposed change awaiting signatures
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    /// Unique identifier for the proposal
    pub proposal_id: u64,
    /// Type of change being proposed
    pub proposal_type: ProposalType,
    /// Address that created the proposal
    pub proposer: Address,
    /// Addresses that have approved this proposal
    pub approvals: Vec<Address>,
    /// Timestamp when proposal was created
    pub created_at: u64,
    /// Timestamp when proposal expires (optional)
    pub expires_at: u64,
    /// Whether the proposal has been executed
    pub executed: bool,
}

/// Storage keys for the Event Registry contract.
#[contracttype]
pub enum DataKey {
    /// The administrator address for contract management (legacy, kept for backward compatibility)
    Admin,
    /// Multi-signature configuration
    MultiSigConfig,
    /// The platform wallet address for fee collection
    PlatformWallet,
    /// The global platform fee percentage
    PlatformFee,
    /// Initialization flag
    Initialized,
    /// Mapping of event_id to EventInfo (Persistent)
    Event(String),
    /// Mapping of organizer_address to a list of their event_ids (Persistent)
    OrganizerEvents(Address),
    /// The authorized TicketPayment contract address for inventory updates
    TicketPaymentContract,
}
