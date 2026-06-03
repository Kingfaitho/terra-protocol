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

// ─── Dispute Enums ────────────────────────────────────────────────────────────

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum DisputeStatus {
    Active,
    Upheld,
    Dismissed,
}

// ─── Dispute Account Structs ──────────────────────────────────────────────────

/// Singleton storing the admin who can resolve disputes.
/// v1: single trusted authority (centrally adjudicated).
/// Future: replace authority with a multisig or on-chain governance key.
/// Initialized once — init constraint prevents re-initialization.
#[account]
pub struct DisputeResolver {
    pub authority: Pubkey, // Admin who can call resolve_dispute
    pub bump: u8,
}

/// An active or resolved dispute against an attested asset.
/// PDA keyed by (asset, disputer) — one dispute per (asset, disputer) pair.
///
/// Bond mechanics (v1 spine):
///   - Upheld:   bond stays locked in this PDA (returned to disputer in Phase 3 Step 2)
///   - Dismissed: bond stays locked in this PDA (sent to treasury in Phase 3 Step 2)
/// Slashed SOL from agents also accumulates here after slash_agent calls.
#[account]
pub struct Dispute {
    pub asset: Pubkey,           // Which asset is disputed (PDA seed)
    pub disputer: Pubkey,        // Who raised the dispute (PDA seed)
    pub evidence_hash: [u8; 32], // hash of disputer's counter-evidence package
    pub bond_amount: u64,        // SOL posted by disputer (locked in this PDA)
    pub status: DisputeStatus,   // Active | Upheld | Dismissed
    pub agents_slashed: u8,      // Running count of agents slashed
    pub total_slashed: u64,      // Total lamports extracted from agents
    pub raised_at: i64,
    pub bump: u8,
}

/// Created by slash_agent to prevent the same agent from being slashed twice
/// in the same dispute. init constraint is the enforcement mechanism.
#[account]
pub struct SlashRecord {
    pub dispute: Pubkey,   // Which dispute triggered this slash
    pub agent: Pubkey,     // Which agent was slashed
    pub slash_amount: u64, // Lamports extracted
    pub bump: u8,
}

/// Treasury singleton PDA. Accumulates 30% of upheld dispute proceeds + dismissed bonds.
/// Authority: multisig or DAO (v2 upgrade path). v1: set once at initialization.
#[account]
pub struct Treasury {
    pub authority: Pubkey, // Who can withdraw from treasury (multisig address or admin)
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

// 8 + 32 authority + 1 bump
pub const DISPUTE_RESOLVER_SIZE: usize = 8 + 32 + 1;

// 8 + 32 asset + 32 disputer + 32 evidence_hash + 8 bond_amount + 1 status
// + 1 agents_slashed + 8 total_slashed + 8 raised_at + 1 bump
pub const DISPUTE_SIZE: usize = 8 + 32 + 32 + 32 + 8 + 1 + 1 + 8 + 8 + 1;

// 8 + 32 dispute + 32 agent + 8 slash_amount + 1 bump
pub const SLASH_RECORD_SIZE: usize = 8 + 32 + 32 + 8 + 1;

// 8 + 32 authority + 1 bump
pub const TREASURY_SIZE: usize = 8 + 32 + 1;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Minimum bond to raise a dispute. Prevents spam (cheap false accusations).
/// 1M lamports = 0.001 SOL. Dismissal sends this to treasury, making griefing expensive.
pub const MIN_DISPUTE_BOND: u64 = 1_000_000;

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

// ─── Dispute Accounts Contexts ────────────────────────────────────────────────

/// One-time setup: creates the singleton admin resolver.
/// Whoever calls this first becomes the resolver — deployer should call immediately.
/// init prevents any future re-initialization.
#[derive(Accounts)]
pub struct InitializeResolver<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = DISPUTE_RESOLVER_SIZE,
        seeds = [b"resolver"],
        bump
    )]
    pub dispute_resolver: Account<'info, DisputeResolver>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RaiseDispute<'info> {
    #[account(mut)]
    pub disputer: Signer<'info>,

    /// Only Verified assets can be disputed.
    /// Constraint checked in instruction body to give a clean error code.
    #[account(
        mut,
        seeds = [b"asset", asset.authority.as_ref(), asset.data_hash.as_ref()],
        bump = asset.bump
    )]
    pub asset: Account<'info, Asset>,

    /// One dispute per (asset, disputer) pair. init prevents duplicate disputes.
    #[account(
        init,
        payer = disputer,
        space = DISPUTE_SIZE,
        seeds = [b"dispute", asset.key().as_ref(), disputer.key().as_ref()],
        bump
    )]
    pub dispute: Account<'info, Dispute>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ResolveDispute<'info> {
    /// Must be the registered admin resolver
    pub resolver: Signer<'info>,

    #[account(
        seeds = [b"resolver"],
        bump = dispute_resolver.bump,
        constraint = dispute_resolver.authority == resolver.key() @ crate::DisputeError::ResolverOnly
    )]
    pub dispute_resolver: Account<'info, DisputeResolver>,

    #[account(
        mut,
        seeds = [b"dispute", dispute.asset.as_ref(), dispute.disputer.as_ref()],
        bump = dispute.bump
    )]
    pub dispute: Account<'info, Dispute>,

    /// Asset whose status changes on resolution.
    /// Key equality to dispute.asset verified via constraint (seeds include data_hash
    /// which the resolver fetches from the existing account before calling).
    #[account(
        mut,
        constraint = asset.key() == dispute.asset @ crate::DisputeError::AssetMismatch
    )]
    pub asset: Account<'info, Asset>,
}

#[derive(Accounts)]
pub struct SlashAgent<'info> {
    /// Anyone can crank this after a dispute is Upheld.
    /// Payer covers the SlashRecord rent (small, ~0.001 SOL).
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"dispute", dispute.asset.as_ref(), dispute.disputer.as_ref()],
        bump = dispute.bump
    )]
    pub dispute: Account<'info, Dispute>,

    /// The agent being slashed
    #[account(
        mut,
        seeds = [b"agent", agent.authority.as_ref()],
        bump = agent.bump
    )]
    pub agent: Account<'info, Agent>,

    /// Proves this agent actually attested the disputed asset.
    /// If this PDA doesn't exist, init below fails — no slashing unrelated agents.
    #[account(
        seeds = [b"attestation", dispute.asset.as_ref(), agent.authority.as_ref()],
        bump = attestation.bump
    )]
    pub attestation: Account<'info, Attestation>,

    /// Created here to prevent double-slashing the same agent in the same dispute.
    #[account(
        init,
        payer = payer,
        space = SLASH_RECORD_SIZE,
        seeds = [b"slash", dispute.key().as_ref(), agent.key().as_ref()],
        bump
    )]
    pub slash_record: Account<'info, SlashRecord>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimUpheldDispute<'info> {
    /// Dispute must be in Upheld status to claim.
    #[account(
        mut,
        seeds = [b"dispute", dispute.asset.as_ref(), dispute.disputer.as_ref()],
        bump = dispute.bump,
        constraint = dispute.status == DisputeStatus::Upheld @ crate::DisputeError::DisputeNotUpheld
    )]
    pub dispute: Account<'info, Dispute>,

    /// The disputer claims their 70% share of (bond + slashes).
    #[account(mut)]
    pub disputer: SystemAccount<'info>,

    /// Treasury receives 30% of (bond + slashes).
    #[account(
        mut,
        seeds = [b"treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, Treasury>,
}

#[derive(Accounts)]
pub struct ClaimDismissedDispute<'info> {
    /// Dispute must be in Dismissed status to claim.
    #[account(
        mut,
        seeds = [b"dispute", dispute.asset.as_ref(), dispute.disputer.as_ref()],
        bump = dispute.bump,
        constraint = dispute.status == DisputeStatus::Dismissed @ crate::DisputeError::DisputeNotDismissed
    )]
    pub dispute: Account<'info, Dispute>,

    /// Treasury receives the full bond amount (disputer gets nothing).
    #[account(
        mut,
        seeds = [b"treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, Treasury>,
}

#[derive(Accounts)]
pub struct InitializeTreasury<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = TREASURY_SIZE,
        seeds = [b"treasury"],
        bump
    )]
    pub treasury: Account<'info, Treasury>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimTreasuryFunds<'info> {
    /// Only the treasury authority can withdraw funds.
    pub authority: Signer<'info>,

    /// Treasury PDA holding the accumulated funds.
    #[account(
        mut,
        seeds = [b"treasury"],
        bump = treasury.bump,
        constraint = treasury.authority == authority.key() @ crate::DisputeError::TreasuryUnauthorized
    )]
    pub treasury: Account<'info, Treasury>,

    /// Recipient of the withdrawn funds (typically authority, but can delegate).
    #[account(mut)]
    pub recipient: SystemAccount<'info>,
}
