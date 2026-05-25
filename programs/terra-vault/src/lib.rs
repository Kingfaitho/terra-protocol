use anchor_lang::prelude::*;

mod state;
use state::*;
declare_id!("5t7Smc2Q4ik9NrR2pr4UhaqPqA1kze1PKwhoFXWBm533");

#[program]
pub mod terra_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.authority = ctx.accounts.authority.key();
        vault.total_deposits = 0;
        vault.total_accrued_interest = 0;
        vault.daily_interest_rate = 800;
        vault.last_interest_accrual = Clock::get()?.unix_timestamp;
        vault.bump = ctx.bumps.vault;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        if amount == 0 { return Err(ProgramError::Custom(1).into()); }
        ctx.accounts.vault_deposit.amount_deposited += amount;
        ctx.accounts.vault.total_deposits += amount;
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        if amount == 0 { return Err(ProgramError::Custom(1).into()); }
        ctx.accounts.vault_deposit.amount_deposited -= amount;
        Ok(())
    }

    pub fn accrue_interest(ctx: Context<AccrueInterest>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        let clock = Clock::get()?;
        let time_elapsed = clock.unix_timestamp - vault.last_interest_accrual;
        if time_elapsed < 86400 { return Err(ProgramError::Custom(1).into()); }
        
        let daily_interest = (vault.total_deposits * vault.daily_interest_rate) / 36500 / 10000;
        vault.total_accrued_interest += daily_interest;
        vault.last_interest_accrual = clock.unix_timestamp;
        Ok(())
    }
}
