use anchor_lang::prelude::*;
use anchor_lang::system_program;

mod state;
use state::*;

declare_id!("5t7Smc2Q4ik9NrR2pr4UhaqPqA1kze1PKwhoFXWBm533");

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
}

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

#[program]
pub mod terra_vault {
    use super::*;

    /// Create a new vault. One vault per authority (PDA ensures uniqueness).
    /// Starts with 8% APY (800 basis points), no deposits, no accrued interest.
    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.total_deposits = 0;
        vault.total_accrued_interest = 0;
        // 800 basis points = 8% annually; daily calc: (deposits * 800) / 36500 / 10000
        vault.daily_interest_rate = 800;
        vault.last_interest_accrual = Clock::get()?.unix_timestamp;
        vault.bump = ctx.bumps.vault;
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

        // Transfer SOL from depositor → vault PDA
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

        // checked_sub gives InsufficientBalance on underflow instead of a panic
        ctx.accounts.vault_deposit.amount_deposited = ctx.accounts.vault_deposit.amount_deposited
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;

        ctx.accounts.vault.total_deposits = ctx.accounts.vault.total_deposits
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;

        // PDAs can't sign for system_program::transfer, so move lamports directly.
        // The vault is owned by this program, so we can reduce its lamports.
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

    /// Accrue daily interest on total vault deposits. May only be called once
    /// per 24 hours. Uses integer-only math to avoid floating-point rounding:
    /// interest = (total_deposits * daily_rate) / 36500 / 10000
    ///
    /// Overflow threshold: ~23 quadrillion lamports (~23M SOL) in a single vault.
    /// checked_mul catches this and returns ArithmeticOverflow instead of a panic.
    pub fn accrue_interest(ctx: Context<AccrueInterest>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        let time_elapsed = clock.unix_timestamp - vault.last_interest_accrual;

        require!(time_elapsed >= 86400, VaultError::AccrualTooSoon);

        // Integer-only: multiply first to preserve precision before dividing.
        // 36500 = days in a year * 100 (annualizes daily rate)
        // 10000 = basis point denominator
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
}
