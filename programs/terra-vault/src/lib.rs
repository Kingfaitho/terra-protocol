use anchor_lang::prelude::*;
use anchor_lang::system_program;

mod state;
use state::*;

declare_id!("5t7Smc2Q4ik9NrR2pr4UhaqPqA1kze1PKwhoFXWBm533");

// ─── Cross-program constants ──────────────────────────────────────────────────

const TERRA_ATTESTATION_ID: &str = "DdzuR1Y9Nmen9XeEC27UJmHeV2oMZhfNLBYww7RBH3Ah";

/// Byte offsets in a terra-attestation Asset account (Borsh layout):
/// 8 disc + 32 authority + 1 asset_type + 8 quantity + 32 loc_hash + 32 data_hash
/// + 1 attestation_count + 1 required_attestations = offset 115
const ASSET_STATUS_OFFSET: usize = 115;
const ASSET_STATUS_VERIFIED: u8 = 1;
const ASSET_STATUS_DISPUTED: u8 = 2;

/// Borsh Option<Pubkey> layout: 1 byte discriminant (0=None, 1=Some) + 32 bytes pubkey
/// linked_vault sits at offset 116 (right after status byte at 115)
const ASSET_LINKED_VAULT_FLAG_OFFSET: usize = 116;
const ASSET_LINKED_VAULT_KEY_OFFSET: usize = 117;
const ASSET_MIN_LEN: usize = ASSET_LINKED_VAULT_KEY_OFFSET + 32; // 149

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

    #[msg("Vault already has an asset gate — remove it first")]
    GateAlreadySet = 6007,

    #[msg("Linked asset must be Disputed before gate can be removed")]
    GateMustBeDisputed = 6008,

    #[msg("Fund amount must be greater than 0")]
    InvalidFundAmount = 6009,
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
    pub principal: u64,
    pub interest: u64,
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

#[event]
pub struct AssetGateRemoved {
    pub vault: Pubkey,
    pub former_asset: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct InterestFunded {
    pub vault: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

// ─── Program ─────────────────────────────────────────────────────────────────

#[program]
pub mod terra_vault {
    use super::*;

    /// Create a new vault. Starts ungated (linked_asset = None).
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

    /// Deposit SOL. Creates a VaultDeposit record and transfers lamports to vault PDA.
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

    /// Withdraw principal + pro-rata interest from the vault.
    ///
    /// Interest payout: depositor receives their fraction of total_accrued_interest,
    /// proportional to the lamports being withdrawn vs. vault total_deposits.
    /// Capped at available excess lamports (vault balance beyond rent + principal).
    /// Interest can only be paid if vault authority has called fund_vault_interest.
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::InvalidDepositAmount);
        require!(
            ctx.accounts.vault_deposit.depositor == ctx.accounts.depositor.key(),
            VaultError::Unauthorized
        );

        let now = Clock::get()?.unix_timestamp;

        // ── Interest calculation ────────────────────────────────────────────
        // Fraction = amount / total_deposits (use u128 to avoid overflow)
        let interest_entitled = if ctx.accounts.vault.total_deposits > 0 {
            (amount as u128)
                .saturating_mul(ctx.accounts.vault.total_accrued_interest as u128)
                .checked_div(ctx.accounts.vault.total_deposits as u128)
                .unwrap_or(0) as u64
        } else {
            0
        };

        // Available interest = vault lamports beyond rent + remaining principal
        let rent = Rent::get()?;
        let vault_rent = rent.minimum_balance(ctx.accounts.vault.to_account_info().data_len());
        let vault_lamports = ctx.accounts.vault.to_account_info().lamports();
        // Remaining principal after this withdrawal
        let remaining_principal = ctx.accounts.vault.total_deposits
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;
        let available_interest = vault_lamports
            .saturating_sub(vault_rent)
            .saturating_sub(remaining_principal)
            .saturating_sub(amount); // exclude the principal we're about to return
        let actual_interest = interest_entitled.min(available_interest);
        // ───────────────────────────────────────────────────────────────────

        ctx.accounts.vault_deposit.amount_deposited = ctx.accounts.vault_deposit.amount_deposited
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;

        ctx.accounts.vault.total_deposits = ctx.accounts.vault.total_deposits
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;

        // Reduce the tracked accrued interest by what we're paying out
        ctx.accounts.vault.total_accrued_interest = ctx.accounts.vault.total_accrued_interest
            .saturating_sub(actual_interest);

        ctx.accounts.vault_deposit.interest_earned = ctx.accounts.vault_deposit.interest_earned
            .checked_add(actual_interest)
            .ok_or(VaultError::ArithmeticOverflow)?;

        let total_payout = amount
            .checked_add(actual_interest)
            .ok_or(VaultError::ArithmeticOverflow)?;

        // PDAs can't sign system_program::transfer — move lamports directly
        **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= total_payout;
        **ctx.accounts.depositor.to_account_info().try_borrow_mut_lamports()? += total_payout;

        emit!(WithdrawalMade {
            vault: ctx.accounts.vault.key(),
            depositor: ctx.accounts.depositor.key(),
            principal: amount,
            interest: actual_interest,
            timestamp: now,
        });

        Ok(())
    }

    /// Accrue daily interest. May only be called once per 24 hours.
    /// If vault.linked_asset is Some, the asset must be Verified.
    pub fn accrue_interest(ctx: Context<AccrueInterest>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        let time_elapsed = clock.unix_timestamp - vault.last_interest_accrual;

        require!(time_elapsed >= 86400, VaultError::AccrualTooSoon);

        if let Some(linked_key) = vault.linked_asset {
            let asset_info = &ctx.accounts.asset;
            require!(asset_info.key() == linked_key, VaultError::InvalidAssetAccount);

            let attestation_id: Pubkey = TERRA_ATTESTATION_ID.parse().unwrap();
            require!(asset_info.owner == &attestation_id, VaultError::InvalidAssetAccount);

            let data = asset_info.try_borrow_data()?;
            require!(data.len() > ASSET_STATUS_OFFSET, VaultError::InvalidAssetAccount);
            require!(data[ASSET_STATUS_OFFSET] == ASSET_STATUS_VERIFIED, VaultError::AssetNotVerified);
        }

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
    /// Enforces a bidirectional check: the asset must have linked_vault == vault.key(),
    /// preventing vaults from gating on assets that were never explicitly linked back.
    /// Can only be set when vault has no current gate (linked_asset == None).
    pub fn set_asset_gate(ctx: Context<SetAssetGate>) -> Result<()> {
        // Prevent overwriting an existing gate — must call remove_asset_gate first
        require!(ctx.accounts.vault.linked_asset.is_none(), VaultError::GateAlreadySet);

        let asset_info = &ctx.accounts.asset;

        let attestation_id: Pubkey = TERRA_ATTESTATION_ID.parse().unwrap();
        require!(asset_info.owner == &attestation_id, VaultError::InvalidAssetAccount);

        let data = asset_info.try_borrow_data()?;
        require!(data.len() >= ASSET_MIN_LEN, VaultError::InvalidAssetAccount);

        // Asset must currently be Verified
        require!(data[ASSET_STATUS_OFFSET] == ASSET_STATUS_VERIFIED, VaultError::AssetNotVerified);

        // Bidirectional check: asset.linked_vault must point back to this vault
        // Prevents gating on assets that never called link_vault(this_vault)
        require!(data[ASSET_LINKED_VAULT_FLAG_OFFSET] == 1, VaultError::InvalidAssetAccount);
        let linked_vault_bytes: [u8; 32] = data[ASSET_LINKED_VAULT_KEY_OFFSET..ASSET_LINKED_VAULT_KEY_OFFSET + 32]
            .try_into()
            .map_err(|_| error!(VaultError::InvalidAssetAccount))?;
        let linked_vault_key = Pubkey::from(linked_vault_bytes);
        require!(linked_vault_key == ctx.accounts.vault.key(), VaultError::InvalidAssetAccount);

        let asset_key = asset_info.key();
        ctx.accounts.vault.linked_asset = Some(asset_key);

        emit!(AssetGateSet {
            vault: ctx.accounts.vault.key(),
            asset: asset_key,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Remove the asset gate when the linked asset is permanently Disputed.
    /// After removal, vault authority can call set_asset_gate with a fresh Verified asset.
    /// Cannot remove a gate on a Verified asset — only Disputed assets release the lock.
    pub fn remove_asset_gate(ctx: Context<RemoveAssetGate>) -> Result<()> {
        let linked_key = ctx.accounts.vault.linked_asset
            .ok_or(error!(VaultError::InvalidAssetAccount))?;

        let asset_info = &ctx.accounts.current_asset;
        require!(asset_info.key() == linked_key, VaultError::InvalidAssetAccount);

        let attestation_id: Pubkey = TERRA_ATTESTATION_ID.parse().unwrap();
        require!(asset_info.owner == &attestation_id, VaultError::InvalidAssetAccount);

        let data = asset_info.try_borrow_data()?;
        require!(data.len() > ASSET_STATUS_OFFSET, VaultError::InvalidAssetAccount);

        // Only allow gate removal when the asset is Disputed (upheld dispute)
        require!(data[ASSET_STATUS_OFFSET] == ASSET_STATUS_DISPUTED, VaultError::GateMustBeDisputed);

        ctx.accounts.vault.linked_asset = None;

        emit!(AssetGateRemoved {
            vault: ctx.accounts.vault.key(),
            former_asset: linked_key,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }

    /// Vault authority deposits SOL specifically to fund interest payouts.
    /// This is the yield source: farmers/SMEs top up the vault from their productive use
    /// of the deposited capital. Without this, withdraw pays principal only.
    pub fn fund_vault_interest(ctx: Context<FundVaultInterest>, amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::InvalidFundAmount);

        let cpi_ctx = CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.authority.to_account_info(),
                to: ctx.accounts.vault.to_account_info(),
            },
        );
        system_program::transfer(cpi_ctx, amount)?;

        emit!(InterestFunded {
            vault: ctx.accounts.vault.key(),
            authority: ctx.accounts.authority.key(),
            amount,
            timestamp: Clock::get()?.unix_timestamp,
        });

        Ok(())
    }
}
