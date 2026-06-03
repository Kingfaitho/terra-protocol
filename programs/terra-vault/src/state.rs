use anchor_lang::prelude::*;

#[account]
pub struct Vault {
    pub authority: Pubkey,
    pub total_deposits: u64,
    pub total_accrued_interest: u64,
    pub daily_interest_rate: u64,
    pub last_interest_accrual: i64,
    pub bump: u8,
    /// When Some, accrue_interest requires the Asset to be Verified and linked back
    /// to this vault (bidirectional). set_asset_gate sets this; remove_asset_gate clears it.
    pub linked_asset: Option<Pubkey>,
    /// Accumulated interest per share (u128 scaled by 1e9 for precision).
    /// Updated in accrue_interest. Used to calculate fair interest for each depositor.
    /// Starts at 0. Incremented as: interest_per_share += (daily_interest * 1e9) / total_deposits
    pub interest_per_share: u128,
}

#[account]
pub struct VaultDeposit {
    pub vault: Pubkey,
    pub depositor: Pubkey,
    pub amount_deposited: u64,
    pub interest_earned: u64, // cumulative interest paid out to this depositor
    pub deposit_timestamp: i64,
    pub bump: u8,
    /// Shares issued when this deposit was created (amount_deposited / share_price at time of deposit).
    /// Used with interest_per_share to calculate fair interest:
    /// interest = (current_interest_per_share - interest_debt) * shares_issued / 1e9
    pub shares_issued: u64,
    /// Interest debt snapshot at deposit time (interest_per_share when account was created).
    /// Subtracted from current interest_per_share to avoid double-counting prior accruals.
    pub interest_debt: u128,
}

// 8 discriminator + 32 authority + 8 total_deposits + 8 total_accrued_interest
// + 8 daily_interest_rate + 8 last_interest_accrual + 1 bump
// + 33 linked_asset (Option<Pubkey>: 1 discriminant + 32 pubkey) + 16 interest_per_share = 122
pub const VAULT_SIZE: usize = 8 + 32 + 8 + 8 + 8 + 8 + 1 + 33 + 16;

// 8 discriminator + 32 vault + 32 depositor + 8 amount_deposited
// + 8 interest_earned + 8 deposit_timestamp + 1 bump + 8 shares_issued + 16 interest_debt = 121
pub const VAULT_DEPOSIT_SIZE: usize = 8 + 32 + 32 + 8 + 8 + 8 + 1 + 8 + 16;

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
    /// Pass any pubkey (e.g. vault key) when vault has no gate — ignored by program.
    /// CHECK: Key verified against vault.linked_asset; owner verified against
    ///        TERRA_ATTESTATION_ID; status byte read at ASSET_STATUS_OFFSET.
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

    /// The terra-attestation Asset to gate this vault.
    /// CHECK: Owner verified; status == Verified; asset.linked_vault == vault.key()
    ///        (bidirectional check — prevents gating on assets that never linked back).
    pub asset: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct RemoveAssetGate<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        constraint = vault.authority == authority.key() @ crate::VaultError::Unauthorized
    )]
    pub vault: Account<'info, Vault>,

    /// The currently linked Asset — must be Disputed to allow gate removal.
    /// CHECK: Key verified against vault.linked_asset; owner and status bytes verified
    ///        in instruction. Gate removal only allowed when asset is Disputed.
    pub current_asset: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct FundVaultInterest<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", authority.key().as_ref()],
        bump = vault.bump,
        constraint = vault.authority == authority.key() @ crate::VaultError::Unauthorized
    )]
    pub vault: Account<'info, Vault>,

    pub system_program: Program<'info, System>,
}
