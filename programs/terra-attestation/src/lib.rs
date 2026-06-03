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
}
