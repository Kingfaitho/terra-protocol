use anchor_lang::prelude::*;
use anchor_lang::system_program;

mod state;
use state::*;

declare_id!("5t7Smc2Q4ik9NrR2pr4UhaqPqA1kze1PKwhoFXWBm533");

// ─── Cross-program constants ──────────────────────────────────────────────────

/// terra-attestation program ID — used to verify ownership of Asset accounts.
const TERRA_ATTESTATION_ID: &str = "DdzuR1Y9Nmen9XeEC27UJmHeV2oMZhfNLBYww7RBH3Ah";

/// Byte offset of the status field inside a terra-attestation Asset account.
/// Layout: 8 discriminator + 32 authority + 1 asset_type + 8 quantity
///       + 32 location_hash + 32 data_hash + 1 attestation_count
///       + 1 required_attestations = 115
const ASSET_STATUS_OFFSET: usize = 115;

/// Borsh enum index for AssetStatus::Verified (index 1 in the AssetStatus enum).
const ASSET_STATUS_VERIFIED: u8 = 1;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[error_code]
pub enum VaultError {
    #[msg("Deposit amount must be greater than 0")]
    InvalidDepositAmount = 6000,

    #[msg("Must wait 24 hours between interest accruals")]
    AccrualTooSoon = 6001,

    #[msg("Insufficient balance to withdraw")]
    InsufficientBalance = 6002,

    #[msg("Only the depositor can withdraw their funds")]
    Unauthorized = 6003,

    #[msg("Arithmetic overflow in vault calculation")]
    ArithmeticOverflow = 6004,

    #[msg("Linked asset is not Verified — interest accrual blocked")]
    AssetNotVerified = 6005,

    #[msg("Provided asset account does not match the vault's linked asset")]
    InvalidAssetAccount = 6006,
}

// ─── Events ───────────────────────────────────────────────────────────────────

#[event]
pub struct DepositMade {
    pub vault: Pubkey,
    pub depositor: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct WithdrawalMade {
    pub vault: Pubkey,
    pub depositor: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct InterestAccrued {
    pub vault: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct AssetGateSet {
    pub vault: Pubkey,
    pub asset: Pubkey,
    pub timestamp: i64,
}

// ─── Program ─────────────────────────────────────────────────────────────────

#[program]
pub mod terra_vault {
    use super::*;

    /// Create a new vault. One vault per authority (PDA ensures uniqueness).
    /// Starts ungated (linked_asset = None) — use set_asset_gate to require attestation.
    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.total_deposits = 0;
        vault.total_accrued_interest = 0;
        // 800 basis points = 8% annually; daily calc: (deposits * 800) / 36500 / 10000
        vault.daily_interest_rate = 800;
        vault.last_interest_accrual = Clock::get()?.unix_timestamp;
        vault.bump = ctx.bumps.vault;
        vault.linked_asset = None;
        Ok(())
    }

    /// Deposit SOL into the vault. Transfers lamports from depositor to vault
    /// and creates a VaultDeposit record tracking the depositor's principal.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::InvalidDepositAmount);

        let now = Clock::get()?.unix_timestamp;

        let vault_deposit = &mut ctx.accounts.vault_deposit;
        vault_deposit.vault = ctx.accounts.vault.key();
        vault_deposit.depositor = ctx.accounts.depositor.key();
        vault_deposit.amount_deposited = amount;
        vault_deposit.interest_earned = 0;
        vault_deposit.deposit_timestamp = now;
        vault_deposit.bump = ctx.bumps.vault_deposit;

        ctx.accounts.vault.total_deposits = ctx.accounts.vault.total_deposits
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.depositor.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, amount)?;

        emit!(DepositMade {
            vault: ctx.accounts.vault.key(),
            depositor: ctx.accounts.depositor.key(),
            amount,
            timestamp: now,
        });

        Ok(())
    }

    /// Withdraw lamports from the vault. Reduces vault total and transfers
    /// SOL back to the depositor via direct lamport manipulation (PDA owns funds).
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::InvalidDepositAmount);
        require!(
            ctx.accounts.vault_deposit.depositor == ctx.accounts.depositor.key(),
            VaultError::Unauthorized
        );

        let now = Clock::get()?.unix_timestamp;

        ctx.accounts.vault_deposit.amount_deposited = ctx.accounts.vault_deposit.amount_deposited
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;

        ctx.accounts.vault.total_deposits = ctx.accounts.vault.total_deposits
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;

        // PDAs can't sign for system_program::transfer, so move lamports directly.
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx.accounts.depositor.to_account_info().try_borrow_mut_lamports()? += amount;

        emit!(WithdrawalMade {
            vault: ctx.accounts.vault.key(),
            depositor: ctx.accounts.depositor.key(),
            amount,
            timestamp: now,
        });

        Ok(())
    }

    /// Accrue daily interest on total vault deposits. May only be called once per 24 hours.
    ///
    /// If vault.linked_asset is Some, the asset account must be passed and its status
    /// must be Verified (byte 115 == 1). A Pending or Disputed asset blocks accrual —
    /// yield only flows when the backing real-world asset is independently confirmed.
    pub fn accrue_interest(ctx: Context<AccrueInterest>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        let time_elapsed = clock.unix_timestamp - vault.last_interest_accrual;

        require!(time_elapsed >= 86400, VaultError::AccrualTooSoon);

        // ── Attestation gate ─────────────────────────────────────────────────
        // When vault.linked_asset is Some, the asset account is validated.
        // When None, the asset account is accepted but ignored (sentinel pattern).
        if let Some(linked_key) = vault.linked_asset {
            let asset_info = &ctx.accounts.asset;

            // The passed account must be the exact asset we stored
            require!(asset_info.key() == linked_key, VaultError::InvalidAssetAccount);

            // Verify account is owned by terra-attestation (prevents spoofed accounts)
            let attestation_id: Pubkey = TERRA_ATTESTATION_ID.parse().unwrap();
            require!(asset_info.owner == &attestation_id, VaultError::InvalidAssetAccount);

            // Read the status byte at the known Borsh layout offset
            let data = asset_info.try_borrow_data()?;
            require!(data.len() > ASSET_STATUS_OFFSET, VaultError::InvalidAssetAccount);
            require!(data[ASSET_STATUS_OFFSET] == ASSET_STATUS_VERIFIED, VaultError::AssetNotVerified);
        }
        // ─────────────────────────────────────────────────────────────────────

        let interest_numerator = vault.total_deposits
            .checked_mul(vault.daily_interest_rate)
            .ok_or(VaultError::ArithmeticOverflow)?;
        let daily_interest = interest_numerator / 36500 / 10000;

        vault.total_accrued_interest = vault.total_accrued_interest
            .checked_add(daily_interest)
            .ok_or(VaultError::ArithmeticOverflow)?;

        vault.last_interest_accrual = clock.unix_timestamp;

        emit!(InterestAccrued {
            vault: vault.key(),
            amount: daily_interest,
            timestamp: clock.unix_timestamp,
        });

        Ok(())
    }

    /// Link a Verified terra-attestation Asset as this vault's interest gate.
    /// After this call, accrue_interest requires the asset to remain Verified.
    /// Validates the asset is owned by terra-attestation and is currently Verified
    /// before recording it — prevents setting a gate to an unverified asset.
    pub fn set_asset_gate(ctx: Context<SetAssetGate>) -> Result<()> {
        let asset_info = &ctx.accounts.asset;

        // Verify account is owned by terra-attestation
        let attestation_id: Pubkey = TERRA_ATTESTATION_ID.parse().unwrap();
        require!(asset_info.owner == &attestation_id, VaultError::InvalidAssetAccount);

        // Verify the asset is currently Verified
        let data = asset_info.try_borrow_data()?;
        require!(data.len() > ASSET_STATUS_OFFSET, VaultError::InvalidAssetAccount);
        require!(data[ASSET_STATUS_OFFSET] == ASSET_STATUS_VERIFIED, VaultError::AssetNotVerified);

        let asset_key = asset_info.key();
        ctx.accounts.vault.linked_asset = Some(asset_key);

        emit!(AssetGateSet {
            vault: ctx.accounts.vault.key(),
            asset: asset_key,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}
