use anchor_lang::prelude::*;

// ─── Enums ───────────────────────────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum AssetType {
    Crop,
    Land,
    Equipment,
    Livestock,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum AssetStatus {
    Pending,
    Verified,
    /// Reserved for Phase 3 dispute oracle — no instruction sets this yet.
    Disputed,
}

// ─── Account Structs ─────────────────────────────────────────────────────────

/// A registered local verifier with SOL staked as a Sybil-resistance deposit.
/// Stake is custodial: returned on clean unregister. Slashing wired in Phase 3.
#[account]
pub struct Agent {
    pub authority: Pubkey,             // Agent wallet (PDA seed)
    pub stake_amount: u64,             // SOL locked in this PDA
    pub reputation_score: u64,         // Increments per successful attestation
    pub active_attestation_count: u32, // Open attestations; must be 0 to unregister
    pub registered_at: i64,
    pub bump: u8,
}

/// A real-world asset registered by a farmer or SME.
/// PDA is content-addressed via data_hash — same evidence can't be registered twice.
#[account]
pub struct Asset {
    pub authority: Pubkey,            // Farmer/SME wallet (PDA seed)
    pub asset_type: AssetType,        // Crop | Land | Equipment | Livestock
    pub quantity: u64,                // Grams for crops, sq metres for land, units for equipment
    pub location_hash: [u8; 32],      // hash(lat || lon) — verifiable without revealing GPS
    pub data_hash: [u8; 32],          // hash(IPFS CID of full evidence package) (PDA seed)
    pub attestation_count: u8,        // Unique agents who have attested
    pub required_attestations: u8,    // Threshold to reach Verified (1–10, set by farmer)
    pub status: AssetStatus,          // Pending | Verified | Disputed
    pub linked_vault: Option<Pubkey>, // Set once by link_vault — None until verified
    pub created_at: i64,
    pub bump: u8,
}

/// One agent's signed claim on one asset.
/// PDA keyed by (asset, agent) — init constraint enforces no duplicate attestations.
#[account]
pub struct Attestation {
    pub asset: Pubkey,       // Which asset was attested
    pub agent: Pubkey,       // Which agent authority attested it
    pub data_hash: [u8; 32], // hash of agent's independent evidence package
    pub attested_at: i64,
    pub bump: u8,
}

// ─── Space Constants ─────────────────────────────────────────────────────────

// 8 discriminator + 32 authority + 8 stake + 8 reputation + 4 active_count + 8 registered_at + 1 bump
pub const AGENT_SIZE: usize = 8 + 32 + 8 + 8 + 4 + 8 + 1;

// 8 + 32 authority + 1 asset_type + 8 quantity + 32 location_hash + 32 data_hash
// + 1 attestation_count + 1 required + 1 status + 33 linked_vault (Option<Pubkey>) + 8 created_at + 1 bump
pub const ASSET_SIZE: usize = 8 + 32 + 1 + 8 + 32 + 32 + 1 + 1 + 1 + 33 + 8 + 1;

// 8 + 32 asset + 32 agent + 32 data_hash + 8 attested_at + 1 bump
pub const ATTESTATION_SIZE: usize = 8 + 32 + 32 + 32 + 8 + 1;

// ─── Accounts Contexts ────────────────────────────────────────────────────────

#[derive(Accounts)]
pub struct RegisterAgent<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = AGENT_SIZE,
        seeds = [b"agent", authority.key().as_ref()],
        bump
    )]
    pub agent: Account<'info, Agent>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UnregisterAgent<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"agent", authority.key().as_ref()],
        bump = agent.bump,
        close = authority  // returns rent + stake lamports to authority, zeroes account
    )]
    pub agent: Account<'info, Agent>,
}

// data_hash is passed as an instruction arg and used as PDA seed so content-addressing
// works at init time (the account doesn't exist yet, so we can't read asset.data_hash).
#[derive(Accounts)]
#[instruction(asset_type: AssetType, quantity: u64, location_hash: [u8; 32], data_hash: [u8; 32])]
pub struct RegisterAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = ASSET_SIZE,
        seeds = [b"asset", authority.key().as_ref(), data_hash.as_ref()],
        bump
    )]
    pub asset: Account<'info, Asset>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AttestAsset<'info> {
    /// The agent's wallet — must be a registered agent
    #[account(mut)]
    pub agent_authority: Signer<'info>,

    /// Agent PDA — verifies agent is registered and gets reputation/count updated
    #[account(
        mut,
        seeds = [b"agent", agent_authority.key().as_ref()],
        bump = agent.bump
    )]
    pub agent: Account<'info, Agent>,

    /// Asset being attested — verified to be a legitimate terra-attestation PDA
    #[account(
        mut,
        seeds = [b"asset", asset.authority.as_ref(), asset.data_hash.as_ref()],
        bump = asset.bump
    )]
    pub asset: Account<'info, Asset>,

    /// One attestation per (asset, agent) pair — init fails if already exists (no duplicates)
    #[account(
        init,
        payer = agent_authority,
        space = ATTESTATION_SIZE,
        seeds = [b"attestation", asset.key().as_ref(), agent_authority.key().as_ref()],
        bump
    )]
    pub attestation: Account<'info, Attestation>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct LinkVault<'info> {
    /// Only the asset's original authority can link it to a vault
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"asset", authority.key().as_ref(), asset.data_hash.as_ref()],
        bump = asset.bump,
        constraint = asset.authority == authority.key() @ crate::AttestationError::Unauthorized
    )]
    pub asset: Account<'info, Asset>,
}
