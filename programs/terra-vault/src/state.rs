use anchor_lang::prelude::*;

#[account]
pub struct Vault {
    pub authority: Pubkey,
    pub total_deposits: u64,
    pub total_accrued_interest: u64,
    pub daily_interest_rate: u64,
    pub last_interest_accrual: i64,
    pub bump: u8,
}

#[account]
pub struct VaultDeposit {
    pub vault: Pubkey,
    pub depositor: Pubkey,
    pub amount_deposited: u64,
    pub interest_earned: u64,
    pub deposit_timestamp: i64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + 32 + 8 + 8 + 8 + 8 + 1,
        seeds = [b"vault", authority.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub vault: Account<'info, Vault>,

    #[account(mut)]
    pub vault_deposit: Account<'info, VaultDeposit>,

    #[account(mut)]
    pub depositor: Signer<'info>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub vault: Account<'info, Vault>,

    #[account(mut)]
    pub vault_deposit: Account<'info, VaultDeposit>,

    #[account(mut)]
    pub depositor: Signer<'info>,
}

#[derive(Accounts)]
pub struct AccrueInterest<'info> {
    #[account(mut)]
    pub vault: Account<'info, Vault>,
}
