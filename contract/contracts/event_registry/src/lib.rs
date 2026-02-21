#![no_std]

use crate::events::{
    AdminAddedEvent, AdminRemovedEvent, AgoraEvent, EventRegisteredEvent, EventStatusUpdatedEvent,
    FeeUpdatedEvent, InitializationEvent, InventoryIncrementedEvent, MetadataUpdatedEvent,
    ProposalApprovedEvent, ProposalCreatedEvent, ProposalExecutedEvent, RegistryUpgradedEvent,
    ThresholdUpdatedEvent,
};
use crate::types::{EventInfo, PaymentInfo, TicketTier};
use soroban_sdk::{contract, contractimpl, Address, BytesN, Env, Map, String, Vec};

pub mod error;
pub mod events;
pub mod storage;
pub mod types;

use crate::error::EventRegistryError;

#[contract]
pub struct EventRegistry;

#[contractimpl]
#[allow(deprecated)]
impl EventRegistry {
    /// Initializes the contract configuration. Can only be called once.
    /// Sets up initial admin with multi-sig configuration (threshold = 1 for single admin).
    ///
    /// # Arguments
    /// * `admin` - The administrator address.
    /// * `platform_wallet` - The platform wallet address for fees.
    /// * `platform_fee_percent` - Initial platform fee in basis points (10000 = 100%).
    pub fn initialize(
        env: Env,
        admin: Address,
        platform_wallet: Address,
        platform_fee_percent: u32,
    ) -> Result<(), EventRegistryError> {
        if storage::is_initialized(&env) {
            return Err(EventRegistryError::AlreadyInitialized);
        }

        validate_address(&env, &admin)?;
        validate_address(&env, &platform_wallet)?;

        let initial_fee = if platform_fee_percent == 0 {
            500
        } else {
            platform_fee_percent
        };

        if initial_fee > 10000 {
            return Err(EventRegistryError::InvalidFeePercent);
        }

        // Initialize multi-sig with single admin and threshold of 1
        let mut admins = Vec::new(&env);
        admins.push_back(admin.clone());
        let multisig_config = MultiSigConfig {
            admins,
            threshold: 1,
        };

        storage::set_admin(&env, &admin); // Legacy support
        storage::set_multisig_config(&env, &multisig_config);
        storage::set_platform_wallet(&env, &platform_wallet);
        storage::set_platform_fee(&env, initial_fee);
        storage::set_initialized(&env, true);

        env.events().publish(
            (AgoraEvent::ContractInitialized,),
            InitializationEvent {
                admin_address: admin,
                platform_wallet,
                platform_fee_percent: initial_fee,
                timestamp: env.ledger().timestamp(),
            },
        );
        Ok(())
    }

    /// Register a new event with organizer authentication and tiered pricing
    ///
    /// # Arguments
    /// * `event_id` - Unique identifier for the event
    /// * `organizer_address` - The wallet address of the event organizer
    /// * `payment_address` - The address where payments should be routed
    /// * `metadata_cid` - IPFS CID for event metadata
    /// * `max_supply` - Maximum number of tickets (0 = unlimited)
    /// * `tiers` - Map of tier_id to TicketTier for multi-tiered pricing
    pub fn register_event(
        env: Env,
        event_id: String,
        organizer_address: Address,
        payment_address: Address,
        metadata_cid: String,
        max_supply: i128,
        tiers: Map<String, TicketTier>,
    ) -> Result<(), EventRegistryError> {
        if !storage::is_initialized(&env) {
            return Err(EventRegistryError::NotInitialized);
        }
        organizer_address.require_auth();

        validate_metadata_cid(&env, &metadata_cid)?;

        if storage::event_exists(&env, event_id.clone()) {
            return Err(EventRegistryError::EventAlreadyExists);
        }

        // Validate tier limits don't exceed max_supply
        if max_supply > 0 {
            let mut total_tier_limit: i128 = 0;
            for tier in tiers.values() {
                total_tier_limit = total_tier_limit
                    .checked_add(tier.tier_limit)
                    .ok_or(EventRegistryError::SupplyOverflow)?;
            }
            if total_tier_limit > max_supply {
                return Err(EventRegistryError::TierLimitExceedsMaxSupply);
            }
        }

        let platform_fee_percent = storage::get_platform_fee(&env);

        let event_info = EventInfo {
            event_id: event_id.clone(),
            organizer_address: organizer_address.clone(),
            payment_address: payment_address.clone(),
            platform_fee_percent,
            is_active: true,
            created_at: env.ledger().timestamp(),
            metadata_cid,
            max_supply,
            current_supply: 0,
            tiers,
        };

        storage::store_event(&env, event_info);

        env.events().publish(
            (AgoraEvent::EventRegistered,),
            EventRegisteredEvent {
                event_id: event_id.clone(),
                organizer_address: organizer_address.clone(),
                payment_address: payment_address.clone(),
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Get event payment information including tiered pricing
    pub fn get_event_payment_info(
        env: Env,
        event_id: String,
    ) -> Result<PaymentInfo, EventRegistryError> {
        match storage::get_event(&env, event_id) {
            Some(event_info) => {
                if !event_info.is_active {
                    return Err(EventRegistryError::EventInactive);
                }
                Ok(PaymentInfo {
                    payment_address: event_info.payment_address,
                    platform_fee_percent: event_info.platform_fee_percent,
                    tiers: event_info.tiers,
                })
            }
            None => Err(EventRegistryError::EventNotFound),
        }
    }

    /// Update event status (only by organizer)
    pub fn update_event_status(
        env: Env,
        event_id: String,
        is_active: bool,
    ) -> Result<(), EventRegistryError> {
        match storage::get_event(&env, event_id.clone()) {
            Some(mut event_info) => {
                // Verify organizer signature
                event_info.organizer_address.require_auth();

                // Update status
                event_info.is_active = is_active;
                storage::store_event(&env, event_info.clone());

                // Emit status update event using contract event type
                env.events().publish(
                    (AgoraEvent::EventStatusUpdated,),
                    EventStatusUpdatedEvent {
                        event_id,
                        is_active,
                        updated_by: event_info.organizer_address,
                        timestamp: env.ledger().timestamp(),
                    },
                );

                Ok(())
            }
            None => Err(EventRegistryError::EventNotFound),
        }
    }

    /// Update the decentralized metadata CID for an event (only by organizer)
    pub fn update_metadata(
        env: Env,
        event_id: String,
        new_metadata_cid: String,
    ) -> Result<(), EventRegistryError> {
        match storage::get_event(&env, event_id.clone()) {
            Some(mut event_info) => {
                // Verify organizer signature
                event_info.organizer_address.require_auth();

                // Validate new metadata CID
                validate_metadata_cid(&env, &new_metadata_cid)?;

                // Update metadata
                event_info.metadata_cid = new_metadata_cid.clone();
                storage::store_event(&env, event_info.clone());

                // Emit metadata update event
                env.events().publish(
                    (AgoraEvent::MetadataUpdated,),
                    MetadataUpdatedEvent {
                        event_id,
                        new_metadata_cid,
                        updated_by: event_info.organizer_address,
                        timestamp: env.ledger().timestamp(),
                    },
                );

                Ok(())
            }
            None => Err(EventRegistryError::EventNotFound),
        }
    }

    /// Stores or updates an event (legacy function for backward compatibility).
    pub fn store_event(env: Env, event_info: EventInfo) {
        // In a real scenario, we would check authorization here.
        storage::store_event(&env, event_info);
    }

    /// Retrieves an event by its ID.
    pub fn get_event(env: Env, event_id: String) -> Option<EventInfo> {
        storage::get_event(&env, event_id)
    }

    /// Checks if an event exists.
    pub fn event_exists(env: Env, event_id: String) -> bool {
        storage::event_exists(&env, event_id)
    }

    /// Retrieves all event IDs for an organizer.
    pub fn get_organizer_events(env: Env, organizer: Address) -> Vec<String> {
        storage::get_organizer_events(&env, &organizer)
    }

    /// Updates the platform fee percentage. Only callable by the administrator.
    pub fn set_platform_fee(env: Env, new_fee_percent: u32) -> Result<(), EventRegistryError> {
        let admin = storage::get_admin(&env).ok_or(EventRegistryError::NotInitialized)?;
        admin.require_auth();

        if new_fee_percent > 10000 {
            return Err(EventRegistryError::InvalidFeePercent);
        }

        storage::set_platform_fee(&env, new_fee_percent);

        // Emit fee update event using contract event type
        env.events().publish(
            (AgoraEvent::FeeUpdated,),
            FeeUpdatedEvent { new_fee_percent },
        );

        Ok(())
    }

    /// Returns the current platform fee percentage.
    pub fn get_platform_fee(env: Env) -> u32 {
        storage::get_platform_fee(&env)
    }

    /// Returns the current administrator address.
    pub fn get_admin(env: Env) -> Result<Address, EventRegistryError> {
        storage::get_admin(&env).ok_or(EventRegistryError::NotInitialized)
    }

    /// Returns the current platform wallet address.
    pub fn get_platform_wallet(env: Env) -> Result<Address, EventRegistryError> {
        storage::get_platform_wallet(&env).ok_or(EventRegistryError::NotInitialized)
    }

    /// Sets the authorized TicketPayment contract address. Only callable by the administrator.
    ///
    /// # Arguments
    /// * `ticket_payment_address` - The address of the TicketPayment contract authorized
    ///   to call `increment_inventory`.
    pub fn set_ticket_payment_contract(
        env: Env,
        ticket_payment_address: Address,
    ) -> Result<(), EventRegistryError> {
        let admin = storage::get_admin(&env).ok_or(EventRegistryError::NotInitialized)?;
        admin.require_auth();

        validate_address(&env, &ticket_payment_address)?;

        storage::set_ticket_payment_contract(&env, &ticket_payment_address);
        Ok(())
    }

    /// Returns the authorized TicketPayment contract address.
    pub fn get_ticket_payment_contract(env: Env) -> Result<Address, EventRegistryError> {
        storage::get_ticket_payment_contract(&env).ok_or(EventRegistryError::NotInitialized)
    }

    /// Increments the current_supply counter for a given event and tier.
    /// This function is restricted to calls from the authorized TicketPayment contract.
    ///
    /// # Arguments
    /// * `event_id` - The event whose inventory to increment.
    /// * `tier_id` - The tier whose inventory to increment.
    ///
    /// # Errors
    /// * `UnauthorizedCaller` - If the invoker is not the registered TicketPayment contract.
    /// * `EventNotFound` - If no event with the given ID exists.
    /// * `EventInactive` - If the event is not currently active.
    /// * `TierNotFound` - If the tier does not exist.
    /// * `TierSupplyExceeded` - If the tier's limit has been reached.
    /// * `MaxSupplyExceeded` - If the event's max supply has been reached (when max_supply > 0).
    /// * `SupplyOverflow` - If incrementing would cause an i128 overflow.
    pub fn increment_inventory(
        env: Env,
        event_id: String,
        tier_id: String,
    ) -> Result<(), EventRegistryError> {
        let ticket_payment_addr =
            storage::get_ticket_payment_contract(&env).ok_or(EventRegistryError::NotInitialized)?;
        ticket_payment_addr.require_auth();

        let mut event_info =
            storage::get_event(&env, event_id.clone()).ok_or(EventRegistryError::EventNotFound)?;

        if !event_info.is_active {
            return Err(EventRegistryError::EventInactive);
        }

        // Check global supply limits
        if event_info.max_supply > 0 && event_info.current_supply >= event_info.max_supply {
            return Err(EventRegistryError::MaxSupplyExceeded);
        }

        // Get and update tier
        let mut tier = event_info
            .tiers
            .get(tier_id.clone())
            .ok_or(EventRegistryError::TierNotFound)?;

        if tier.current_sold >= tier.tier_limit {
            return Err(EventRegistryError::TierSupplyExceeded);
        }

        tier.current_sold = tier
            .current_sold
            .checked_add(1)
            .ok_or(EventRegistryError::SupplyOverflow)?;

        event_info.tiers.set(tier_id, tier);

        event_info.current_supply = event_info
            .current_supply
            .checked_add(1)
            .ok_or(EventRegistryError::SupplyOverflow)?;

        storage::store_event(&env, event_info.clone());

        env.events().publish(
            (AgoraEvent::InventoryIncremented,),
            InventoryIncrementedEvent {
                event_id,
                new_supply: event_info.current_supply,
                max_supply: event_info.max_supply,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Decrements the current_supply counter for a given event and tier.
    /// This function is restricted to calls from the authorized TicketPayment contract upon refund.
    ///
    /// # Arguments
    /// * `event_id` - The event whose inventory to decrement.
    /// * `tier_id` - The tier whose inventory to decrement.
    ///
    /// # Errors
    /// * `UnauthorizedCaller` - If the invoker is not the registered TicketPayment contract.
    /// * `EventNotFound` - If no event with the given ID exists.
    /// * `TierNotFound` - If the tier does not exist.
    /// * `SupplyUnderflow` - If decrementing would cause the supply to go below 0.
    pub fn decrement_inventory(
        env: Env,
        event_id: String,
        tier_id: String,
    ) -> Result<(), EventRegistryError> {
        let ticket_payment_addr =
            storage::get_ticket_payment_contract(&env).ok_or(EventRegistryError::NotInitialized)?;
        ticket_payment_addr.require_auth();

        let mut event_info =
            storage::get_event(&env, event_id.clone()).ok_or(EventRegistryError::EventNotFound)?;

        // Get and update tier
        let mut tier = event_info
            .tiers
            .get(tier_id.clone())
            .ok_or(EventRegistryError::TierNotFound)?;

        if tier.current_sold <= 0 {
            return Err(EventRegistryError::SupplyUnderflow);
        }

        tier.current_sold = tier
            .current_sold
            .checked_sub(1)
            .ok_or(EventRegistryError::SupplyUnderflow)?;

        event_info.tiers.set(tier_id, tier);

        if event_info.current_supply <= 0 {
            return Err(EventRegistryError::SupplyUnderflow);
        }

        event_info.current_supply = event_info
            .current_supply
            .checked_sub(1)
            .ok_or(EventRegistryError::SupplyUnderflow)?;

        storage::store_event(&env, event_info.clone());

        env.events().publish(
            (crate::events::AgoraEvent::InventoryDecremented,),
            crate::events::InventoryDecrementedEvent {
                event_id,
                new_supply: event_info.current_supply,
                max_supply: event_info.max_supply,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Upgrades the contract to a new WASM hash. Only callable by the administrator.
    /// Performs post-upgrade state verification to ensure critical storage is intact.
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) -> Result<(), EventRegistryError> {
        let admin = storage::get_admin(&env).ok_or(EventRegistryError::NotInitialized)?;
        admin.require_auth();

        env.deployer().update_current_contract_wasm(new_wasm_hash);

        // Post-upgrade state verification
        let verified_admin = storage::get_admin(&env).ok_or(EventRegistryError::NotInitialized)?;
        storage::get_platform_wallet(&env).ok_or(EventRegistryError::NotInitialized)?;

        env.events().publish(
            (AgoraEvent::ContractUpgraded,),
            RegistryUpgradedEvent {
                admin_address: verified_admin,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    // ==================== MULTI-SIG GOVERNANCE FUNCTIONS ====================

    /// Creates a proposal for changing sensitive platform parameters.
    /// Any admin can create a proposal.
    ///
    /// # Arguments
    /// * `proposer` - The admin creating the proposal
    /// * `proposal_type` - The type of change being proposed
    /// * `expiration_ledgers` - Number of ledgers until proposal expires (0 = no expiration)
    pub fn create_proposal(
        env: Env,
        proposer: Address,
        proposal_type: ProposalType,
        expiration_ledgers: u32,
    ) -> Result<u64, EventRegistryError> {
        if !storage::is_initialized(&env) {
            return Err(EventRegistryError::NotInitialized);
        }

        // Verify proposer is an admin
        proposer.require_auth();
        if !storage::is_admin(&env, &proposer) {
            return Err(EventRegistryError::Unauthorized);
        }

        // Validate proposal type
        validate_proposal_type(&env, &proposal_type)?;

        let proposal_id = storage::get_next_proposal_id(&env);
        let created_at = env.ledger().timestamp();
        let expires_at = if expiration_ledgers > 0 {
            created_at + (expiration_ledgers as u64 * 5) // Approximate 5 seconds per ledger
        } else {
            u64::MAX // No expiration
        };

        let mut approvals = Vec::new(&env);
        approvals.push_back(proposer.clone()); // Proposer automatically approves

        let proposal = Proposal {
            proposal_id,
            proposal_type,
            proposer: proposer.clone(),
            approvals,
            created_at,
            expires_at,
            executed: false,
        };

        storage::store_proposal(&env, &proposal);

        env.events().publish(
            (AgoraEvent::ProposalCreated,),
            ProposalCreatedEvent {
                proposal_id,
                proposer,
                timestamp: created_at,
            },
        );

        Ok(proposal_id)
    }

    /// Approves a proposal. Each admin can approve once.
    ///
    /// # Arguments
    /// * `approver` - The admin approving the proposal
    /// * `proposal_id` - The ID of the proposal to approve
    pub fn approve_proposal(
        env: Env,
        approver: Address,
        proposal_id: u64,
    ) -> Result<(), EventRegistryError> {
        approver.require_auth();

        if !storage::is_admin(&env, &approver) {
            return Err(EventRegistryError::Unauthorized);
        }

        let mut proposal =
            storage::get_proposal(&env, proposal_id).ok_or(EventRegistryError::ProposalNotFound)?;

        if proposal.executed {
            return Err(EventRegistryError::ProposalAlreadyExecuted);
        }

        // Check expiration
        if proposal.expires_at != u64::MAX && env.ledger().timestamp() > proposal.expires_at {
            return Err(EventRegistryError::ProposalExpired);
        }

        // Check if already approved
        for approval in proposal.approvals.iter() {
            if approval == approver {
                return Err(EventRegistryError::AlreadyApproved);
            }
        }

        proposal.approvals.push_back(approver.clone());
        storage::store_proposal(&env, &proposal);

        env.events().publish(
            (AgoraEvent::ProposalApproved,),
            ProposalApprovedEvent {
                proposal_id,
                approver,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Executes a proposal if it has sufficient approvals.
    ///
    /// # Arguments
    /// * `executor` - The admin executing the proposal
    /// * `proposal_id` - The ID of the proposal to execute
    pub fn execute_proposal(
        env: Env,
        executor: Address,
        proposal_id: u64,
    ) -> Result<(), EventRegistryError> {
        executor.require_auth();

        if !storage::is_admin(&env, &executor) {
            return Err(EventRegistryError::Unauthorized);
        }

        let mut proposal =
            storage::get_proposal(&env, proposal_id).ok_or(EventRegistryError::ProposalNotFound)?;

        if proposal.executed {
            return Err(EventRegistryError::ProposalAlreadyExecuted);
        }

        // Check expiration
        if proposal.expires_at != u64::MAX && env.ledger().timestamp() > proposal.expires_at {
            return Err(EventRegistryError::ProposalExpired);
        }

        let config =
            storage::get_multisig_config(&env).ok_or(EventRegistryError::NotInitialized)?;

        // Check if proposal has sufficient approvals
        if (proposal.approvals.len() as u32) < config.threshold {
            return Err(EventRegistryError::InsufficientApprovals);
        }

        // Execute the proposal based on type
        match &proposal.proposal_type {
            ProposalType::SetPlatformWallet(new_wallet) => {
                validate_address(&env, new_wallet)?;
                storage::set_platform_wallet(&env, new_wallet);
            }
            ProposalType::AddAdmin(new_admin) => {
                validate_address(&env, new_admin)?;
                add_admin_internal(&env, new_admin)?;
                env.events().publish(
                    (AgoraEvent::AdminAdded,),
                    AdminAddedEvent {
                        admin: new_admin.clone(),
                        added_by: executor.clone(),
                        timestamp: env.ledger().timestamp(),
                    },
                );
            }
            ProposalType::RemoveAdmin(admin_to_remove) => {
                remove_admin_internal(&env, admin_to_remove)?;
                env.events().publish(
                    (AgoraEvent::AdminRemoved,),
                    AdminRemovedEvent {
                        admin: admin_to_remove.clone(),
                        removed_by: executor.clone(),
                        timestamp: env.ledger().timestamp(),
                    },
                );
            }
            ProposalType::SetThreshold(new_threshold) => {
                set_threshold_internal(&env, *new_threshold)?;
                env.events().publish(
                    (AgoraEvent::ThresholdUpdated,),
                    ThresholdUpdatedEvent {
                        old_threshold: config.threshold,
                        new_threshold: *new_threshold,
                        timestamp: env.ledger().timestamp(),
                    },
                );
            }
        }

        // Mark proposal as executed
        proposal.executed = true;
        storage::store_proposal(&env, &proposal);
        storage::remove_from_active_proposals(&env, proposal_id);

        env.events().publish(
            (AgoraEvent::ProposalExecuted,),
            ProposalExecutedEvent {
                proposal_id,
                executor,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Retrieves a proposal by ID.
    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, EventRegistryError> {
        storage::get_proposal(&env, proposal_id).ok_or(EventRegistryError::ProposalNotFound)
    }

    /// Retrieves all active proposal IDs.
    pub fn get_active_proposals(env: Env) -> Vec<u64> {
        storage::get_active_proposals(&env)
    }

    /// Retrieves the current multi-sig configuration.
    pub fn get_multisig_config(env: Env) -> Result<MultiSigConfig, EventRegistryError> {
        storage::get_multisig_config(&env).ok_or(EventRegistryError::NotInitialized)
    }

    /// Checks if an address is an admin.
    pub fn is_admin(env: Env, address: Address) -> bool {
        storage::is_admin(&env, &address)
    }

    /// Proposes to change the platform wallet (requires multi-sig approval).
    /// This is a convenience function that creates a proposal.
    pub fn propose_set_platform_wallet(
        env: Env,
        proposer: Address,
        new_wallet: Address,
        expiration_ledgers: u32,
    ) -> Result<u64, EventRegistryError> {
        Self::create_proposal(
            env,
            proposer,
            ProposalType::SetPlatformWallet(new_wallet),
            expiration_ledgers,
        )
    }

    /// Proposes to add a new admin (requires multi-sig approval).
    pub fn propose_add_admin(
        env: Env,
        proposer: Address,
        new_admin: Address,
        expiration_ledgers: u32,
    ) -> Result<u64, EventRegistryError> {
        Self::create_proposal(
            env,
            proposer,
            ProposalType::AddAdmin(new_admin),
            expiration_ledgers,
        )
    }

    /// Proposes to remove an admin (requires multi-sig approval).
    pub fn propose_remove_admin(
        env: Env,
        proposer: Address,
        admin_to_remove: Address,
        expiration_ledgers: u32,
    ) -> Result<u64, EventRegistryError> {
        Self::create_proposal(
            env,
            proposer,
            ProposalType::RemoveAdmin(admin_to_remove),
            expiration_ledgers,
        )
    }

    /// Proposes to change the signature threshold (requires multi-sig approval).
    pub fn propose_set_threshold(
        env: Env,
        proposer: Address,
        new_threshold: u32,
        expiration_ledgers: u32,
    ) -> Result<u64, EventRegistryError> {
        Self::create_proposal(
            env,
            proposer,
            ProposalType::SetThreshold(new_threshold),
            expiration_ledgers,
        )
    }
}

fn validate_address(env: &Env, address: &Address) -> Result<(), EventRegistryError> {
    if address == &env.current_contract_address() {
        return Err(EventRegistryError::InvalidAddress);
    }
    Ok(())
}

fn validate_metadata_cid(env: &Env, cid: &String) -> Result<(), EventRegistryError> {
    if cid.len() < 46 {
        return Err(EventRegistryError::InvalidMetadataCid);
    }

    // We expect CIDv1 base32, which starts with 'b'
    // Convert to Bytes to check the first character safely
    let mut bytes = soroban_sdk::Bytes::new(env);
    bytes.append(&cid.clone().into());

    if !bytes.is_empty() && bytes.get(0) != Some(b'b') {
        return Err(EventRegistryError::InvalidMetadataCid);
    }

    Ok(())
}

fn validate_proposal_type(
    env: &Env,
    proposal_type: &ProposalType,
) -> Result<(), EventRegistryError> {
    match proposal_type {
        ProposalType::SetPlatformWallet(wallet) => validate_address(env, wallet),
        ProposalType::AddAdmin(admin) => {
            validate_address(env, admin)?;
            // Check if admin already exists
            if storage::is_admin(env, admin) {
                return Err(EventRegistryError::AdminAlreadyExists);
            }
            Ok(())
        }
        ProposalType::RemoveAdmin(admin) => {
            // Check if admin exists
            if !storage::is_admin(env, admin) {
                return Err(EventRegistryError::AdminNotFound);
            }
            // Check if this would remove the last admin
            let config =
                storage::get_multisig_config(env).ok_or(EventRegistryError::NotInitialized)?;
            if config.admins.len() <= 1 {
                return Err(EventRegistryError::CannotRemoveLastAdmin);
            }
            Ok(())
        }
        ProposalType::SetThreshold(threshold) => {
            let config =
                storage::get_multisig_config(env).ok_or(EventRegistryError::NotInitialized)?;
            if *threshold == 0 || *threshold > config.admins.len() as u32 {
                return Err(EventRegistryError::InvalidThreshold);
            }
            Ok(())
        }
    }
}

fn add_admin_internal(env: &Env, new_admin: &Address) -> Result<(), EventRegistryError> {
    let mut config = storage::get_multisig_config(env).ok_or(EventRegistryError::NotInitialized)?;

    // Check if admin already exists
    for admin in config.admins.iter() {
        if admin == *new_admin {
            return Err(EventRegistryError::AdminAlreadyExists);
        }
    }

    config.admins.push_back(new_admin.clone());
    storage::set_multisig_config(env, &config);
    Ok(())
}

fn remove_admin_internal(env: &Env, admin_to_remove: &Address) -> Result<(), EventRegistryError> {
    let mut config = storage::get_multisig_config(env).ok_or(EventRegistryError::NotInitialized)?;

    // Prevent removing the last admin
    if config.admins.len() <= 1 {
        return Err(EventRegistryError::CannotRemoveLastAdmin);
    }

    let mut new_admins = Vec::new(env);
    let mut found = false;

    for admin in config.admins.iter() {
        if admin == *admin_to_remove {
            found = true;
        } else {
            new_admins.push_back(admin);
        }
    }

    if !found {
        return Err(EventRegistryError::AdminNotFound);
    }

    config.admins = new_admins;

    // Adjust threshold if it exceeds the new admin count
    if config.threshold > config.admins.len() as u32 {
        config.threshold = config.admins.len() as u32;
    }

    storage::set_multisig_config(env, &config);
    Ok(())
}

fn set_threshold_internal(env: &Env, new_threshold: u32) -> Result<(), EventRegistryError> {
    let mut config = storage::get_multisig_config(env).ok_or(EventRegistryError::NotInitialized)?;

    if new_threshold == 0 || new_threshold > config.admins.len() as u32 {
        return Err(EventRegistryError::InvalidThreshold);
    }

    config.threshold = new_threshold;
    storage::set_multisig_config(env, &config);
    Ok(())
}

#[cfg(test)]
mod test;

#[cfg(test)]
mod test_multisig;
