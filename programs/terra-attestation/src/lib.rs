use anchor_lang::prelude::*;
use anchor_lang::system_program;

mod state;
use state::*;

declare_id!("DdzuR1Y9Nmen9XeEC27UJmHeV2oMZhfNLBYww7RBH3Ah");

// ─── Errors ───────────────────────────────────────────────────────────────────

#[error_code]
pub enum AttestationError {
    #[msg("Stake amount must be greater than 0")]
    InvalidStakeAmount = 6100,

    #[msg("required_attestations must be between 1 and 10")]
    InvalidRequiredAttestations = 6101,

    #[msg("Asset has not reached the required attestation threshold")]
    AssetNotVerified = 6102,

    #[msg("Asset is already linked to a vault")]
    AlreadyLinked = 6103,

    #[msg("Only the asset authority can perform this action")]
    Unauthorized = 6104,

    #[msg("Agent still has open attestations — unregister not allowed")]
    AgentHasOpenAttestations = 6105,

    #[msg("Arithmetic overflow in attestation calculation")]
    ArithmeticOverflow = 6106,
}

/// Dispute-specific errors in a separate enum to keep ranges distinct.
/// Range: 6200+ (distinct from AttestationError 6100–6106)
#[error_code]
pub enum DisputeError {
    #[msg("Only a Verified asset can be disputed")]
    AssetNotVerifiable = 6200,

    #[msg("Dispute must be Active to resolve")]
    DisputeNotActive = 6201,

    #[msg("Dispute must be Upheld to slash agents")]
    DisputeNotUpheld = 6202,

    #[msg("Only the registered resolver admin can resolve disputes")]
    ResolverOnly = 6203,

    #[msg("Bond amount must be greater than 0")]
    InvalidBondAmount = 6204,

    #[msg("Agent has no remaining stake to slash")]
    AgentStakeExhausted = 6205,

    #[msg("Asset does not match the dispute record")]
    AssetMismatch = 6206,

    #[msg("Dispute must be Dismissed to claim bond refund")]
    DisputeNotDismissed = 6207,

    #[msg("Bond amount must be at least the minimum (1M lamports = 0.001 SOL)")]
    BondTooSmall = 6208,
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[event]
pub struct AgentRegistered {
    pub agent: Pubkey,
    pub authority: Pubkey,
    pub stake_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct AgentUnregistered {
    pub agent: Pubkey,
    pub authority: Pubkey,
    pub stake_returned: u64,
    pub timestamp: i64,
}

#[event]
pub struct AssetRegistered {
    pub asset: Pubkey,
    pub authority: Pubkey,
    pub asset_type: AssetType,
    pub required_attestations: u8,
    pub timestamp: i64,
}

#[event]
pub struct AssetAttested {
    pub asset: Pubkey,
    pub agent: Pubkey,
    pub attestation_count: u8,
    pub status: AssetStatus,
    pub timestamp: i64,
}

#[event]
pub struct AssetLinkedToVault {
    pub asset: Pubkey,
    pub vault: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct DisputeRaised {
    pub dispute: Pubkey,
    pub asset: Pubkey,
    pub disputer: Pubkey,
    pub bond_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct DisputeResolved {
    pub dispute: Pubkey,
    pub asset: Pubkey,
    pub upheld: bool,
    pub timestamp: i64,
}

#[event]
pub struct AgentSlashed {
    pub dispute: Pubkey,
    pub agent: Pubkey,
    pub slash_amount: u64,
    pub remaining_stake: u64,
    pub timestamp: i64,
}

#[event]
pub struct DisputeRewardClaimed {
    pub dispute: Pubkey,
    pub disputer: Pubkey,
    pub disputer_share: u64,
    pub treasury_share: u64,
    pub timestamp: i64,
}

#[event]
pub struct DismissedBondForfeited {
    pub dispute: Pubkey,
    pub bond_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct TreasuryInitialized {
    pub treasury: Pubkey,
    pub authority: Pubkey,
    pub timestamp: i64,
}

// ─── Program ─────────────────────────────────────────────────────────────────

#[program]
pub mod terra_attestation {
    use super::*;

    /// Register as an attestation agent by locking SOL as a Sybil-resistance deposit.
    /// Stake is custodial — returned in full on clean unregister.
    /// Purpose: making it costly to spin up multiple fake agent identities.
    pub fn register_agent(ctx: Context<RegisterAgent>, stake_amount: u64) -> Result<()> {
        require!(stake_amount > 0, AttestationError::InvalidStakeAmount);

        let now = Clock::get()?.unix_timestamp;

        let agent = &mut ctx.accounts.agent;
        agent.authority = ctx.accounts.authority.key();
        agent.stake_amount = stake_amount;
        agent.reputation_score = 0;
        agent.active_attestation_count = 0;
        agent.registered_at = now;
        agent.bump = ctx.bumps.agent;

        // Lock stake: transfer SOL from authority → agent PDA
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.agent.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, stake_amount)?;

        emit!(AgentRegistered {
            agent: ctx.accounts.agent.key(),
            authority: ctx.accounts.authority.key(),
            stake_amount,
            timestamp: now,
        });

        Ok(())
    }

    /// Unregister as an agent, returning all locked SOL (rent + stake) to authority.
    /// Blocked if agent has open attestations — ensures no orphaned records.
    pub fn unregister_agent(ctx: Context<UnregisterAgent>) -> Result<()> {
        require!(
            ctx.accounts.agent.active_attestation_count == 0,
            AttestationError::AgentHasOpenAttestations
        );

        let now = Clock::get()?.unix_timestamp;
        let stake_returned = ctx.accounts.agent.stake_amount;

        // The `close = authority` constraint in state.rs transfers all PDA lamports
        // (rent-exempt minimum + stake_amount) back to authority and zeroes the account.

        emit!(AgentUnregistered {
            agent: ctx.accounts.agent.key(),
            authority: ctx.accounts.authority.key(),
            stake_returned,
            timestamp: now,
        });

        Ok(())
    }

    /// Register a real-world asset for attestation.
    /// The asset PDA is content-addressed via data_hash — registering identical
    /// evidence twice is impossible (same PDA, init fails).
    pub fn register_asset(
        ctx: Context<RegisterAsset>,
        asset_type: AssetType,
        quantity: u64,
        location_hash: [u8; 32],
        data_hash: [u8; 32],
        required_attestations: u8,
    ) -> Result<()> {
        require!(
            required_attestations >= 1 && required_attestations <= 10,
            AttestationError::InvalidRequiredAttestations
        );

        let now = Clock::get()?.unix_timestamp;

        let asset = &mut ctx.accounts.asset;
        asset.authority = ctx.accounts.authority.key();
        asset.asset_type = asset_type.clone();
        asset.quantity = quantity;
        asset.location_hash = location_hash;
        asset.data_hash = data_hash;
        asset.attestation_count = 0;
        asset.required_attestations = required_attestations;
        asset.status = AssetStatus::Pending;
        asset.linked_vault = None;
        asset.created_at = now;
        asset.bump = ctx.bumps.asset;

        emit!(AssetRegistered {
            asset: ctx.accounts.asset.key(),
            authority: ctx.accounts.authority.key(),
            asset_type,
            required_attestations,
            timestamp: now,
        });

        Ok(())
    }

    /// Submit an independent attestation for an asset.
    /// The Attestation PDA is keyed by (asset, agent_authority) — Anchor's init
    /// constraint naturally prevents any agent from attesting the same asset twice.
    /// When attestation_count reaches required_attestations the asset flips to Verified.
    pub fn attest_asset(ctx: Context<AttestAsset>, data_hash: [u8; 32]) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;

        // Record the attestation
        let attestation = &mut ctx.accounts.attestation;
        attestation.asset = ctx.accounts.asset.key();
        attestation.agent = ctx.accounts.agent_authority.key();
        attestation.data_hash = data_hash;
        attestation.attested_at = now;
        attestation.bump = ctx.bumps.attestation;

        // Update asset — checked_add guards against count wrapping past 255
        let asset = &mut ctx.accounts.asset;
        asset.attestation_count = asset
            .attestation_count
            .checked_add(1)
            .ok_or(AttestationError::ArithmeticOverflow)?;

        if asset.attestation_count >= asset.required_attestations {
            asset.status = AssetStatus::Verified;
        }

        // Update agent reputation and open count
        let agent = &mut ctx.accounts.agent;
        agent.reputation_score = agent
            .reputation_score
            .checked_add(1)
            .ok_or(AttestationError::ArithmeticOverflow)?;
        agent.active_attestation_count = agent
            .active_attestation_count
            .checked_add(1)
            .ok_or(AttestationError::ArithmeticOverflow)?;

        emit!(AssetAttested {
            asset: asset.key(),
            agent: ctx.accounts.agent_authority.key(),
            attestation_count: asset.attestation_count,
            status: asset.status.clone(),
            timestamp: now,
        });

        Ok(())
    }

    /// Link a Verified asset to a vault, enabling attestation-gated tokenization.
    /// Only the asset's original authority can link it.
    /// Cross-program vault validation (CPI to terra-vault) is Phase 2 Step 2.
    pub fn link_vault(ctx: Context<LinkVault>, vault: Pubkey) -> Result<()> {
        require!(
            ctx.accounts.asset.status == AssetStatus::Verified,
            AttestationError::AssetNotVerified
        );
        require!(
            ctx.accounts.asset.linked_vault.is_none(),
            AttestationError::AlreadyLinked
        );

        let now = Clock::get()?.unix_timestamp;

        ctx.accounts.asset.linked_vault = Some(vault);

        emit!(AssetLinkedToVault {
            asset: ctx.accounts.asset.key(),
            vault,
            timestamp: now,
        });

        Ok(())
    }

    // ─── Dispute instructions ──────────────────────────────────────────────────

    /// One-time setup: register the admin resolver.
    /// v1: centrally adjudicated. Caller becomes the authority.
    /// Deployer must call this immediately after deployment to prevent front-running.
    pub fn initialize_resolver(ctx: Context<InitializeResolver>) -> Result<()> {
        let dr = &mut ctx.accounts.dispute_resolver;
        dr.authority = ctx.accounts.authority.key();
        dr.bump = ctx.bumps.dispute_resolver;
        Ok(())
    }

    /// Raise a dispute against a Verified asset.
    /// Posting a bond signals economic commitment — spam disputes cost real SOL.
    /// The asset immediately flips to Disputed, pausing any linked vault's interest.
    /// Bond must meet minimum (1M lamports) to prevent griefing.
    pub fn raise_dispute(
        ctx: Context<RaiseDispute>,
        evidence_hash: [u8; 32],
        bond_amount: u64,
    ) -> Result<()> {
        require!(bond_amount >= MIN_DISPUTE_BOND, DisputeError::BondTooSmall);
        require!(
            ctx.accounts.asset.status == AssetStatus::Verified,
            DisputeError::AssetNotVerifiable
        );

        let now = Clock::get()?.unix_timestamp;

        let dispute = &mut ctx.accounts.dispute;
        dispute.asset = ctx.accounts.asset.key();
        dispute.disputer = ctx.accounts.disputer.key();
        dispute.evidence_hash = evidence_hash;
        dispute.bond_amount = bond_amount;
        dispute.status = DisputeStatus::Active;
        dispute.agents_slashed = 0;
        dispute.total_slashed = 0;
        dispute.raised_at = now;
        dispute.bump = ctx.bumps.dispute;

        // Transfer bond: disputer → dispute PDA (locked until resolution)
        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.disputer.to_account_info(),
                to: ctx.accounts.dispute.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, bond_amount)?;

        // Flip asset to Disputed — vault gate fires automatically (status != Verified)
        ctx.accounts.asset.status = AssetStatus::Disputed;

        emit!(DisputeRaised {
            dispute: ctx.accounts.dispute.key(),
            asset: ctx.accounts.asset.key(),
            disputer: ctx.accounts.disputer.key(),
            bond_amount,
            timestamp: now,
        });

        Ok(())
    }

    /// Admin resolver upholds or dismisses the dispute.
    /// Upheld  → asset stays Disputed; slash_agent can now be called per attester.
    /// Dismissed → asset reverts to Verified; interest resumes immediately.
    /// v1: single trusted authority. Bond + slashed SOL distribution is Phase 3 Step 2.
    pub fn resolve_dispute(ctx: Context<ResolveDispute>, upheld: bool) -> Result<()> {
        require!(
            ctx.accounts.dispute.status == DisputeStatus::Active,
            DisputeError::DisputeNotActive
        );

        let now = Clock::get()?.unix_timestamp;

        if upheld {
            ctx.accounts.dispute.status = DisputeStatus::Upheld;
            // Asset remains Disputed — vault interest stays blocked until Phase 3 Step 2
            // handles aftermath (new valid attestations or asset retirement).
        } else {
            ctx.accounts.dispute.status = DisputeStatus::Dismissed;
            // Revert to Verified — vault interest gate clears automatically
            ctx.accounts.asset.status = AssetStatus::Verified;
        }

        emit!(DisputeResolved {
            dispute: ctx.accounts.dispute.key(),
            asset: ctx.accounts.asset.key(),
            upheld,
            timestamp: now,
        });

        Ok(())
    }

    /// Slash one attesting agent after a dispute is Upheld.
    /// Anyone can crank this — the SlashRecord PDA (via init) prevents double-execution.
    ///
    /// Slash amount: 50% of the agent's current stake (floor: 0 if already exhausted).
    ///
    /// Slash-evasion invariant: active_attestation_count can only reach 0 through
    /// slashing. An agent who ever attested an asset cannot unregister until they've
    /// been slashed in every upheld dispute against those assets. This is intentional:
    /// decrementing active_attestation_count via any other path re-opens stake withdrawal.
    pub fn slash_agent(ctx: Context<SlashAgent>) -> Result<()> {
        require!(
            ctx.accounts.dispute.status == DisputeStatus::Upheld,
            DisputeError::DisputeNotUpheld
        );
        require!(
            ctx.accounts.agent.stake_amount > 0,
            DisputeError::AgentStakeExhausted
        );

        let now = Clock::get()?.unix_timestamp;

        let slash_amount = ctx.accounts.agent.stake_amount / 2;

        // Record the slash (prevents double-slashing in the same dispute)
        let slash_record = &mut ctx.accounts.slash_record;
        slash_record.dispute = ctx.accounts.dispute.key();
        slash_record.agent = ctx.accounts.agent.key();
        slash_record.slash_amount = slash_amount;
        slash_record.bump = ctx.bumps.slash_record;

        // Move lamports: Agent PDA → Dispute PDA
        // Agent PDA is owned by this program, so direct lamport manipulation is valid.
        **ctx.accounts.agent.to_account_info().try_borrow_mut_lamports()? -= slash_amount;
        **ctx.accounts.dispute.to_account_info().try_borrow_mut_lamports()? += slash_amount;

        // Reduce stored stake amount
        ctx.accounts.agent.stake_amount = ctx.accounts.agent.stake_amount
            .checked_sub(slash_amount)
            .ok_or(AttestationError::ArithmeticOverflow)?;

        // Decrement active count — the slash terminates this attestation commitment.
        // This is the ONLY place active_attestation_count decrements; see invariant note above.
        ctx.accounts.agent.active_attestation_count = ctx.accounts.agent.active_attestation_count
            .checked_sub(1)
            .ok_or(AttestationError::ArithmeticOverflow)?;

        // Accumulate dispute totals
        ctx.accounts.dispute.total_slashed = ctx.accounts.dispute.total_slashed
            .checked_add(slash_amount)
            .ok_or(AttestationError::ArithmeticOverflow)?;
        ctx.accounts.dispute.agents_slashed = ctx.accounts.dispute.agents_slashed
            .checked_add(1)
            .ok_or(AttestationError::ArithmeticOverflow)?;

        let remaining_stake = ctx.accounts.agent.stake_amount;

        emit!(AgentSlashed {
            dispute: ctx.accounts.dispute.key(),
            agent: ctx.accounts.agent.key(),
            slash_amount,
            remaining_stake,
            timestamp: now,
        });

        Ok(())
    }

    // ─── Reward distribution (Phase 3 Step 2) ──────────────────────────────────

    /// Initialize the treasury singleton. v1: set once at deployment.
    /// Authority can be a multisig (for v2 upgrade path).
    pub fn initialize_treasury(ctx: Context<InitializeTreasury>) -> Result<()> {
        let treasury = &mut ctx.accounts.treasury;
        treasury.authority = ctx.accounts.authority.key();
        treasury.bump = ctx.bumps.treasury;

        emit!(TreasuryInitialized {
            treasury: ctx.accounts.treasury.key(),
            authority: ctx.accounts.authority.key(),
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Claim reward from an Upheld dispute: 70% to disputer, 30% to treasury.
    /// Total payout = bond_amount + total_slashed.
    /// Only callable when dispute status is Upheld.
    pub fn claim_upheld_dispute(ctx: Context<ClaimUpheldDispute>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let dispute = &mut ctx.accounts.dispute;

        let total_pool = dispute.bond_amount
            .checked_add(dispute.total_slashed)
            .ok_or(AttestationError::ArithmeticOverflow)?;

        // 70% to disputer, 30% to treasury
        let disputer_share = (total_pool * 70u64) / 100u64;
        let treasury_share = total_pool.saturating_sub(disputer_share);

        // Transfer disputer share
        **dispute.to_account_info().try_borrow_mut_lamports()? -= disputer_share;
        **ctx.accounts.disputer.to_account_info().try_borrow_mut_lamports()? += disputer_share;

        // Transfer treasury share
        **dispute.to_account_info().try_borrow_mut_lamports()? -= treasury_share;
        **ctx.accounts.treasury.to_account_info().try_borrow_mut_lamports()? += treasury_share;

        emit!(DisputeRewardClaimed {
            dispute: dispute.key(),
            disputer: ctx.accounts.disputer.key(),
            disputer_share,
            treasury_share,
            timestamp: now,
        });

        Ok(())
    }

    /// Claim from a Dismissed dispute: 100% of bond goes to treasury.
    /// Disputer gets nothing — false accusations are expensive.
    /// Only callable when dispute status is Dismissed.
    pub fn claim_dismissed_dispute(ctx: Context<ClaimDismissedDispute>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let dispute = &mut ctx.accounts.dispute;

        let bond = dispute.bond_amount;

        // Transfer bond to treasury
        **dispute.to_account_info().try_borrow_mut_lamports()? -= bond;
        **ctx.accounts.treasury.to_account_info().try_borrow_mut_lamports()? += bond;

        emit!(DismissedBondForfeited {
            dispute: dispute.key(),
            bond_amount: bond,
            timestamp: now,
        });

        Ok(())
    }
}
