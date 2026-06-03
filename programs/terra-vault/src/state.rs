use anchor_lang::prelude::*;

#[account]
pub struct Vault {
    pub authority: Pubkey,
    pub total_deposits: u64,
    pub total_accrued_interest: u64,
    pub daily_interest_rate: u64,
    pub last_interest_accrual: i64,
    pub bump: u8,
    /// Set by set_asset_gate. When Some, accrue_interest requires the referenced
    /// terra-attestation Asset account to have status == Verified.
    /// None means the vault accrues interest unconditionally (no gate).
    pub linked_asset: Option<Pubkey>,
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

// 8 discriminator + 32 authority + 8 total_deposits + 8 total_accrued_interest
// + 8 daily_interest_rate + 8 last_interest_accrual + 1 bump
// + 33 linked_asset (Option<Pubkey>: 1 discriminant + 32 pubkey) = 106
pub const VAULT_SIZE: usize = 8 + 32 + 8 + 8 + 8 + 8 + 1 + 33;

// 8 discriminator + 32 vault + 32 depositor + 8 amount_deposited
// + 8 interest_earned + 8 deposit_timestamp + 1 bump = 97
pub const VAULT_DEPOSIT_SIZE: usize = 8 + 32 + 32 + 8 + 8 + 8 + 1;

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = VAULT_SIZE,
        seeds = [b"vault", authority.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        mut,
        seeds = [b"vault", vault.authority.as_ref()],
        bump = vault.bump
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        init,
        payer = depositor,
        space = VAULT_DEPOSIT_SIZE,
        seeds = [b"deposit", vault.key().as_ref(), depositor.key().as_ref()],
        bump
    )]
    pub vault_deposit: Account<'info, VaultDeposit>,

    #[account(mut)]
    pub depositor: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(
        mut,
        seeds = [b"vault", vault.authority.as_ref()],
        bump = vault.bump
    )]
    pub vault: Account<'info, Vault>,

    #[account(
        mut,
        seeds = [b"deposit", vault.key().as_ref(), depositor.key().as_ref()],
        bump = vault_deposit.bump
    )]
    pub vault_deposit: Account<'info, VaultDeposit>,

    #[account(mut)]
    pub depositor: Signer<'info>,
}

#[derive(Accounts)]
pub struct AccrueInterest<'info> {
    #[account(
        mut,
        seeds = [b"vault", vault.authority.as_ref()],
        bump = vault.bump
    )]
    pub vault: Account<'info, Vault>,

    /// Pass the terra-attestation Asset PDA when vault.linked_asset is Some.
    /// Pass any pubkey (e.g. vault key itself) when vault has no gate — it is ignored.
    /// CHECK: Key verified against vault.linked_asset; owner verified against
    ///        TERRA_ATTESTATION_ID; status byte read at ASSET_STATUS_OFFSET.
    ///        Only validated when vault.linked_asset is Some.
    pub asset: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SetAssetGate<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        constraint = vault.authority == authority.key() @ crate::VaultError::Unauthorized
    )]
    pub vault: Account<'info, Vault>,

    /// The terra-attestation Asset account to use as an interest gate.
    /// CHECK: Owner verified against TERRA_ATTESTATION_ID in instruction logic.
    ///        Status byte verified as Verified (1) before storing the pubkey.
    pub asset: UncheckedAccount<'info>,
}
