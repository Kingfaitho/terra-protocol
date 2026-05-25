use anchor_lang::prelude::*;
use crate::state::*;

// Initialize a new vault
pub fn initialize_vault(
    ctx: Context<InitializeVault>,
    name: [u8; 50],
    asset_description: [u8; 200],
) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    
    vault.authority = ctx.accounts.authority.key();
    vault.name = name;
    vault.asset_description = asset_description;
    vault.total_deposits = 0;
    vault.total_accrued_interest = 0;
    vault.daily_interest_rate = 800; // 8% annually = ~0.022% daily
    vault.last_interest_accrual = Clock::get()?.unix_timestamp;
    vault.bump = ctx.bumps.vault;
    
    Ok(())
}

// Deposit money into the vault
pub fn deposit(
    ctx: Context<Deposit>,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, "Deposit amount must be greater than 0");
    
    let vault = &mut ctx.accounts.vault;
    let vault_deposit = &mut ctx.accounts.vault_deposit;
    let clock = Clock::get()?;
    
    // Update vault deposit record
    if vault_deposit.amount_deposited == 0 {
        // First deposit
        vault_deposit.vault = vault.key();
        vault_deposit.depositor = ctx.accounts.depositor.key();
        vault_deposit.deposit_timestamp = clock.unix_timestamp;
        vault_deposit.bump = ctx.bumps.vault_deposit;
    }
    
    vault_deposit.amount_deposited += amount;
    vault.total_deposits += amount;
    
    // Transfer SOL (or token) from depositor to vault
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.depositor.to_account_info(),
                to: vault.to_account_info(),
            },
        ),
        amount,
    )?;
    
    msg!("Deposited {} lamports into vault", amount);
    Ok(())
}

// Withdraw money from the vault
pub fn withdraw(
    ctx: Context<Withdraw>,
    amount: u64,
) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    let vault_deposit = &mut ctx.accounts.vault_deposit;
    
    let total_available = vault_deposit.amount_deposited + vault_deposit.interest_earned;
    require!(amount <= total_available, "Insufficient balance to withdraw");
    
    // Calculate how much is principal vs interest
    let principal_withdrawal = if amount <= vault_deposit.amount_deposited {
        amount
    } else {
        vault_deposit.amount_deposited
    };
    
    let interest_withdrawal = amount.saturating_sub(principal_withdrawal);
    
    vault_deposit.amount_deposited -= principal_withdrawal;
    vault_deposit.interest_earned -= interest_withdrawal;
    vault.total_deposits -= principal_withdrawal;
    
    // Transfer back to depositor
    **vault.to_account_info().lamports.borrow_mut() -= amount;
    **ctx.accounts.depositor.to_account_info().lamports.borrow_mut() += amount;
    
    msg!("Withdrawn {} lamports from vault", amount);
    Ok(())
}

// Accrue daily interest (NO FLOATING POINT — all integer math)
pub fn accrue_interest(
    ctx: Context<AccrueInterest>,
) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    let clock = Clock::get()?;
    
    let time_elapsed = clock.unix_timestamp - vault.last_interest_accrual;
    require!(time_elapsed >= 86400, "Interest can only accrue once per day"); // 86400 seconds = 1 day
    
    // PRECISION MATH (no floats):
    // If daily_interest_rate = 800 (meaning 800 basis points = 8% annually)
    // Daily rate = 800 / 36500 ≈ 0.0219% per day
    // Interest = total_deposits * daily_rate / 10000
    
    let daily_interest_earned = (vault.total_deposits * (vault.daily_interest_rate as u64)) / 36500 / 10000;
    
    vault.total_accrued_interest += daily_interest_earned;
    vault.last_interest_accrual = clock.unix_timestamp;
    
    msg!("Interest accrued: {} lamports", daily_interest_earned);
    Ok(())
}
