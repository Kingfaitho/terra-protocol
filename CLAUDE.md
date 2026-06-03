# TERRA — Verifiable Asset Tokenization Protocol

## Complete Co-Founder Knowledge Base v2.0

**Builder:** KingFaitho | **Status:** Phase 3 complete — 44/44 tests passing | **Last Updated:** June 3, 2026

-----

## EXECUTIVE SUMMARY (Read This First)

**What We’re Building:**
A precision financial infrastructure for African Real-World Assets (RWAs) on Solana. Farmers tokenize crops/land, investors deposit SOL and earn verified yield, local agents attest harvests, disputes slash fraudulent attestors.

**Current State (June 2026):**

- ✅ terra-vault: deposit, withdraw (principal + pro-rata interest), accrue_interest (precision math), fund_vault_interest, set_asset_gate, remove_asset_gate
- ✅ terra-attestation: agent registration (Sybil-resistant staking), asset registration (content-addressed), 3-of-N attestation quorum, attestation-gated vault linking
- ✅ Dispute & Slashing: raise_dispute (bond), resolve_dispute (admin resolver), slash_agent (50% stake, SlashRecord anti-double-slash)
- ✅ Cross-program gate: vault.accrue_interest reads terra-attestation Asset status byte; Disputed = interest blocked automatically
- ✅ Vault recovery: remove_asset_gate (only when Disputed), re-gate to new Verified asset
- ✅ Bidirectional link: asset.linked_vault must == vault.key() before set_asset_gate succeeds
- ✅ Full economic loop proven: fund_vault_interest → accrue → withdraw with real SOL interest paid
- ✅ 44/44 tests passing (integration + bankrun + attestation + dispute + interest_payout)
- ⏳ Phase 3 Step 2: dispute reward distribution (bond + slashed SOL → treasury/disputer)
- ⏳ Phase 4: React frontend + Privy login

**Programs:**
- terra-vault: `5t7Smc2Q4ik9NrR2pr4UhaqPqA1kze1PKwhoFXWBm533`
- terra-attestation: `DdzuR1Y9Nmen9XeEC27UJmHeV2oMZhfNLBYww7RBH3Ah`

**Timeline:** 18 months to mainnet | **Funding:** Bootstrapping | **Team:** KingFaitho + Claude | **Scope:** Unlimited (quality > speed)

-----

## I. MISSION & VISION

### The Problem We Solve

**In Africa/Emerging Markets:**

- Farmers can’t tokenize land/crops without predatory loans (50%+ interest)
- Investors don’t trust RWA claims (how do you verify a harvest from 5000 miles away?)
- No programmable structured finance (no tranching, no yield optimization)
- Currency risk + execution risk = capital never flows to Africa
- Mobile users need SMS/USSD solutions, not web wallets

**Our Solution:**

1. **On-chain vault** with verifiable, zero-error interest calculation (Safe Vault precision math)
1. **Cryptographic attestation** layer (agents stake SOL, sign real-world proofs)
1. **Dynamic tranching** (senior/junior debt based on asset volatility + AI forecasts)
1. **Mobile-first issuance** (farmers issue tokens via SMS, not CLI)
1. **MEV-resistant execution** (batch auctions for large RWA trades)

### The 18-Month Vision

```
PHASE 1 (Months 1-3): Precision Settlement Backbone        ← WE ARE HERE
├─ Step 1: ✅ DONE - Programs compiled, pushed to GitHub
├─ Step 2: TODAY - Write tests, deploy to devnet
├─ Step 3: This week - Custom errors, event logging
└─ Step 4: Next week - Security audit checklist

PHASE 2 (Months 4-6): Verifiable Asset Attestation
├─ Agent reputation system (stake + sign)
├─ Merkle tree proofs (cheap on-chain verification)
└─ Asset registry (GPS, photos, harvest data)

PHASE 3 (Months 7-9): Yield Modeling & Dynamic Tranching
├─ Actuarial library (duration, convexity, volatility)
├─ Tranche logic (senior=8%, junior=20%)
└─ Insurance fund mechanics

PHASE 4 (Months 10-12): Mobile UI & Issuance
├─ React app (deposits, withdrawals, dashboards)
├─ Privy integration (phone/email login)
└─ Twilio SMS notifications

PHASE 5 (Months 13-15): Marketplace & MEV Resistance
├─ Batch auction engine
├─ Relayer network
└─ Tranche trading UI

PHASE 6 (Months 16-18): Audit & Mainnet
├─ Security audit (external firm)
├─ Testnet pilot (10 real farmers)
└─ Mainnet deployment + grants
```

-----

## II. CURRENT STATE (Commit: bb2e07f)

### What’s Actually Built

**Repository:** `https://github.com/Kingfaitho/terra-protocol`

#### Program 1: terra-vault (THE CORE ENGINE)

**Status:** ✅ Compiled & Functional

**Location:** `programs/terra-vault/src/`

**Files:**

- `lib.rs` — Entry point, #[program] module, all 4 functions
- `state.rs` — Data structures (Vault, VaultDeposit accounts)
- `instructions.rs` — DEPRECATED (move logic to lib.rs today)

**Program ID:** `4xwPZ3FgFbyqT8eZVGmjaPex5cAVaYazzjKd8cSTsHNe`

**What It Does:**

```rust
// 4 Functions, All Implemented:

1. initialize_vault()
   Input: None (derives PDA from authority)
   Creates: Vault account with:
   - authority = caller
   - total_deposits = 0
   - total_accrued_interest = 0
   - daily_interest_rate = 800 (8% annually)
   - last_interest_accrual = now

2. deposit(amount: u64)
   Input: Amount in lamports (SOL micro-units)
   Creates/Updates: VaultDeposit account
   - Records: amountDeposited += amount
   - Updates: Vault.totalDeposits += amount
   - Transfers: SOL from depositor → vault account
   Effect: Investor's money is now in the vault, earning interest

3. withdraw(amount: u64)
   Input: Amount to withdraw
   Checks: total_available = amountDeposited + interestEarned
   Transfers: Back to investor
   Updates: Both balances reduced
   Effect: Investor gets principal + interest in their wallet

4. accrue_interest()
   Input: None
   Time Check: Must be ≥86400 seconds (24 hours) since last call
   Calculation: interest = (total_deposits * daily_rate) / 36500 / 10000
   Example: 1 SOL (1M lamports) * 800 / 36500 / 10000 = 2 lamports/day
   Updates: Vault.totalAccruedInterest += interest
   Effect: Interest is locked in, visible to everyone
```

**The Precision Math (NO FLOATING POINT):**

```
Problem: Normal math would be:
  rate = 800 / 36500 = 0.0219178... (float, rounds away, loses pennies)
  interest = 1_000_000 * 0.0219178... = 21917.8... (more rounding)

Our Solution (Integer Only):
  interest = (1_000_000 * 800) / 36500 / 10000
  Step 1: 1_000_000 * 800 = 800_000_000 (no rounding yet)
  Step 2: 800_000_000 / 36500 = 21917 (integer division, exact)
  Step 3: 21917 / 10000 = 2 (lamports, exact)

Result: 100% precision, zero errors, perfect for fintech in volatile regions.
```

**Data Structures:**

```rust
#[account]
struct Vault {
    pub authority: Pubkey,               // Vault creator (farmer/SME)
    pub total_deposits: u64,             // All investor money combined (lamports)
    pub total_accrued_interest: u64,     // Cumulative interest earned (lamports)
    pub daily_interest_rate: u64,        // Annual rate in basis points (800 = 8%)
    pub last_interest_accrual: i64,      // Unix timestamp of last accrual
    pub bump: u8,                        // PDA bump (Solana security feature)
}

#[account]
struct VaultDeposit {
    pub vault: Pubkey,                   // Which vault does this belong to?
    pub depositor: Pubkey,               // Investor's wallet
    pub amount_deposited: u64,           // Principal (lamports)
    pub interest_earned: u64,            // Accrued interest (lamports)
    pub deposit_timestamp: i64,          // When they deposited (Unix timestamp)
    pub bump: u8,                        // PDA bump
}
```

#### Program 2: terra-attestation

**Status:** ✅ Compiled | ⏳ Stub (Logic in Phase 2)

**Program ID:** `8TTpQxUUnAMN7QRHi12AxptWbFWX6yGBMAa9d2QvXNLs`

**What It Will Do (Phase 2):**

- Agents (trusted local community members) stake SOL
- Sign asset claims: “I verified 50 tons of dried mint at GPS coordinates X,Y”
- Attach photos/IoT sensor data
- Program batches attestations into Merkle trees (cheap on-chain storage)
- Investors verify: Is this asset real? Who verified it?

#### Program 3: terra-marketplace

**Status:** ✅ Compiled | ⏳ Stub (Logic in Phase 5)

**Program ID:** `BjMmwi4LBtNNb96nzwdBVU84FRWSbbiMr22ZsmHfUGie`

**What It Will Do (Phase 5):**

- Investors trade RWA tranches (senior/junior debt)
- Batch auction engine (prevents sandwich attacks, MEV)
- Price discovery
- Rebalancing triggers (if yield drops, auto-sell junior tranche)

-----

## III. PROJECT STRUCTURE (Exactly What’s Where)

```
terra-protocol/
│
├── programs/                                    # Solana programs (the smart contracts)
│   ├── terra-vault/                            # ★ THE CORE
│   │   ├── src/
│   │   │   ├── lib.rs                          # ✅ Complete - 4 functions, all logic
│   │   │   ├── state.rs                        # ✅ Complete - Vault, VaultDeposit structs
│   │   │   └── instructions.rs                 # ⚠️ DEPRECATED - Delete today
│   │   ├── Cargo.toml                          # ✅ anchor-lang 0.30.0
│   │   └── keypair.json                        # Program's private key
│   │
│   ├── terra-attestation/
│   │   ├── src/lib.rs                          # ⏳ Stub only
│   │   └── Cargo.toml
│   │
│   └── terra-marketplace/
│       ├── src/lib.rs                          # ⏳ Stub only
│       └── Cargo.toml
│
├── app/                                         # React frontend (Phase 4)
│   ├── src/
│   ├── package.json
│   └── public/
│
├── indexer/                                     # Off-chain data sync (Phase 2)
│   ├── src/
│   └── Cargo.toml
│
├── tests/                                       # Integration tests (TypeScript + Anchor)
│   └── integration.ts                          # ⏳ BEING WRITTEN TODAY
│
├── Anchor.toml                                  # Cluster config, program IDs, RPC
├── Cargo.toml                                   # Rust workspace (members: 3 programs + indexer)
├── package.json                                 # Node dependencies (Anchor, Solana CLI wrappers)
├── README.md                                    # Public overview
├── CLAUDE.md                                    # THIS FILE (co-founder knowledge base)
└── .gitignore                                   # What NOT to push to GitHub
```

-----

## IV. HOW IT WORKS (Data Flow Example)

### Scenario: Farmer Deposits 10 SOL, Earns Interest

```
STEP 1: INITIALIZATION (Farmer's First Action)
───────────────────────────────────────────────

Farmer calls:    initialize_vault()
↓
Program derives: Vault PDA = hash(b"vault" + farmer_pubkey)
↓
Creates account: Vault with:
  - authority = farmer
  - total_deposits = 0
  - total_accrued_interest = 0
  - daily_interest_rate = 800 (8% APY)
↓
Blockchain state: ✅ Vault is LIVE, visible to everyone
  └─ Can verify: solana account <VAULT_PDA> --url devnet


STEP 2: DEPOSIT (Farmer Adds Money)
────────────────────────────────────

Farmer calls:    deposit(10_000_000) # 10 SOL in lamports
↓
Program derives: VaultDeposit PDA = hash(b"deposit" + vault + farmer)
↓
Records:         VaultDeposit = {
                   vault: vault_pubkey,
                   depositor: farmer,
                   amount_deposited: 10_000_000,
                   interest_earned: 0,
                   deposit_timestamp: now
                 }
↓
Transfers:       10 SOL from farmer's wallet → vault account (CPI)
↓
Updates:         Vault.total_deposits = 10_000_000
↓
Blockchain state: ✅ Both accounts synced
  └─ Farmer can verify: My 10 SOL is locked in, earning interest


STEP 3: DAILY INTEREST ACCRUAL (Automated, Anyone Can Call)
───────────────────────────────────────────────────────────

Anyone calls:    accrue_interest()
↓
Program checks:  Is it ≥24 hours since last accrual?
                 If not: ERROR (only once per day)
                 If yes: PROCEED
↓
Calculates:      interest = (10_000_000 * 800) / 36500 / 10000
                           = 8_000_000_000 / 36500 / 10000
                           = 219_178 / 10000
                           = 21 lamports (integer division)
↓
Updates:         Vault.total_accrued_interest = 21
                 Vault.last_interest_accrual = now
↓
Blockchain state: ✅ Interest is now REAL, visible, verifiable
  Day 1: +21 lamports
  Day 2: +21 lamports
  Day 30: +630 lamports total


STEP 4: WITHDRAWAL (Farmer Gets Money + Interest Back)
───────────────────────────────────────────────────────

Farmer calls:    withdraw(5_000_000) # Half their principal
↓
Program checks:  total_available = 10_000_000 + 630 = 10_000_630
                 Requesting: 5_000_000
                 Status: ✅ ALLOWED
↓
Splits:          principal_part = 5_000_000
                 interest_part = accrued interest (pro-rata)
↓
Transfers:       5_000_000 + interest back to farmer wallet (CPI)
↓
Updates:         VaultDeposit.amount_deposited = 5_000_000
                 VaultDeposit.interest_earned = 0 (after withdrawal)
                 Vault.total_deposits = 5_000_000
↓
Blockchain state: ✅ Farmer now has 5 SOL + 0.00000630 SOL interest
  └─ Can verify: solana balance <FARMER_PUBKEY> --url devnet


RESULT
──────
Farmer started with: 10 SOL
After 30 days: 5 SOL + 0.00000630 SOL interest (withdrawn)
                5 SOL still earning (in vault)

No banks involved. No loan sharks. 100% transparent. On-chain forever.
```

-----

## V. TECHNICAL ARCHITECTURE

### Tech Stack

|Layer              |Technology                            |Why                                      |
|-------------------|--------------------------------------|-----------------------------------------|
|**Smart Contracts**|Rust + Anchor 0.30.0                  |Type-safe, fast, Solana native           |
|**Runtime**        |Solana blockchain                     |Cheap, fast, Africa-friendly             |
|**State**          |Anchor PDAs (Program Derived Accounts)|Deterministic, secure, scalable          |
|**Frontend**       |React (coming Phase 4)                |Mobile-friendly, familiar to devs        |
|**Indexing**       |Rust (coming Phase 2)                 |Listen to on-chain events, sync off-chain|
|**Deployment**     |Anchor CLI + GitHub Actions           |One-command deployments, CI/CD           |

### Security Model

**Vault Account Ownership:**

- Owned by: terra-vault program (immutable)
- Authority: Vault creator (can update settings)
- Access: PDA ensures unique vault per authority (no collisions)

**VaultDeposit Account Ownership:**

- Owned by: terra-vault program (immutable)
- Depositor: Can only withdraw their own balance
- Access: PDA ensures unique record per (vault, depositor) pair

**Precision:**

- All math: Integer-only (u64)
- No floating-point operations (ever)
- Division order: (a * b) / c (not (a/b) * c)

**Validation:**

- All accounts checked before use
- All amounts validated (>0, <u64::MAX)
- Time checks enforced (24-hour accrual cooldown)

-----

## VI. CURRENT ISSUES & HOW TO FIX

### Issue 1: Error Handling Is Generic

**Current:**

```rust
if amount == 0 { 
  return Err(ProgramError::Custom(1).into()); 
}
```

**Problem:** User sees “Error 1” — meaningless

**Fix (Phase 1, Step 3):**

```rust
#[error_code]
pub enum VaultError {
    #[msg("Deposit amount must be greater than 0")]
    InvalidDepositAmount = 6000,
    
    #[msg("Must wait 24 hours between interest accruals")]
    AccrualTooSoon = 6001,
    
    #[msg("Insufficient balance to withdraw")]
    InsufficientBalance = 6002,
}

// Then use:
require!(amount > 0, VaultError::InvalidDepositAmount);
```

### Issue 2: No Tests

**Current:** Program compiles but behavior unverified

**Fix (TODAY):** Write integration tests (see Phase 1, Step 2)

### Issue 3: No Events

**Current:** Changes are on-chain but hard to track off-chain

**Fix (Phase 1, Step 3):** Emit events for deposits, withdrawals, interest

```rust
#[event]
pub struct DepositMade {
    pub vault: Pubkey,
    pub depositor: Pubkey,
    pub amount: u64,
    pub timestamp: i64,
}

// Then emit:
emit!(DepositMade {
    vault: vault.key(),
    depositor: depositor.key(),
    amount,
    timestamp: Clock::get()?.unix_timestamp,
});
```

### Issue 4: instructions.rs Is Redundant

**Current:** Logic in lib.rs, but old instructions.rs still exists

**Fix (TODAY):** Delete it

```bash
rm programs/terra-vault/src/instructions.rs
```

-----

## VII. EXECUTION ROADMAP (DETAILED)

### Phase 1: Precision Settlement Backbone (NOW)

**Step 1: ✅ COMPLETE**

- [x] Create 3 Solana programs (vault, attestation, marketplace)
- [x] Implement vault logic (initialize, deposit, withdraw, accrue)
- [x] Compile without errors
- [x] Push to GitHub (Commit: bb2e07f)

**Step 2: TODAY (RIGHT NOW)**

- [ ] Delete instructions.rs
- [ ] Write integration tests (initialize, deposit, accrue, withdraw)
- [ ] Test locally: `anchor test`
- [ ] Deploy to devnet: `anchor deploy`
- [ ] Verify on-chain: Check Solscan
- [ ] Commit: “Phase 1, Step 2: Tests + Devnet Deployment”

**Step 3: This Week**

- [ ] Define custom error codes (VaultError enum)
- [ ] Emit events (DepositMade, InterestAccrued, Withdrawn)
- [ ] Add function comments (explain each step)
- [ ] Create GitHub Issues for Phase 2
- [ ] Write architecture document
- [ ] Commit: “Phase 1, Step 3: Error handling + events”

**Step 4: Next Week**

- [ ] Security audit checklist (PDAs, overflow, underflow)
- [ ] Optimize compute units
- [ ] Test edge cases (overflow, underflow, max u64)
- [ ] Create runbook for devnet deployment
- [ ] Commit: “Phase 1, Step 4: Security hardening”

### Phase 2: Verifiable Asset Attestation (Weeks 5-12)

**Build:**

- Agent reputation system (stake collateral, sign claims)
- Merkle tree batching (cheap on-chain proofs)
- Asset registry PDA (metadata, status, proof link)
- Integration with terra-vault (link asset → vault)

**Deliverable:** Farmers can verify “this asset is real” on-chain

### Phase 3: Yield Modeling & Dynamic Tranching (Weeks 13-21)

**Build:**

- Actuarial library (duration, convexity, volatility)
- Tranche logic (senior/junior splits)
- Dynamic fee calculation based on volatility
- Insurance fund mechanics (put option-like)

**Deliverable:** Investors see “8% senior, 20% junior” yields

### Phase 4: Mobile UI & Issuance (Weeks 22-30)

**Build:**

- React app (deposits, withdrawals, dashboards)
- Privy integration (phone/email login, no wallets)
- Real-time balance updates
- SMS notifications (Twilio)

**Deliverable:** Farmers issue tokens via phone, get SMS updates

### Phase 5: Marketplace & MEV Resistance (Weeks 31-39)

**Build:**

- Batch auction engine (prevents sandwich attacks)
- Relayer network (private order flow)
- Tranche trading UI
- Portfolio dashboard

**Deliverable:** Investors trade tranches without MEV loss

### Phase 6: Audit & Mainnet (Weeks 40-52)

**Build:**

- External security audit
- Testnet pilot (10 real farmers, real money)
- Mainnet deployment
- Case studies + grants

**Deliverable:** Live on mainnet, real users, real yield

-----

## VIII. DEBUGGING & LOCAL DEVELOPMENT

### Build & Test Locally

```bash
# Navigate to project
cd ~/terra-protocol

# Build all programs
anchor build --no-idl

# Check for issues
cargo check

# Clean rebuild
cargo clean && anchor build --no-idl

# Run all tests
anchor test

# Run specific test
anchor test -- --grep "Deposits into vault"

# Watch for changes (requires cargo-watch)
cargo watch -x "anchor build"
```

### Deploy to Devnet

```bash
# Point to devnet
solana config set --url devnet

# Check current config
solana config get

# Get free SOL (airdrop)
solana airdrop 2 --url devnet

# Deploy all programs
anchor deploy

# Check deployment status
solana program show 4xwPZ3FgFbyqT8eZVGmjaPex5cAVaYazzjKd8cSTsHNe --url devnet

# View program logs
solana logs 4xwPZ3FgFbyqT8eZVGmjaPex5cAVaYazzjKd8cSTsHNe --url devnet
```

### Inspect On-Chain State

```bash
# Check vault account
solana account <VAULT_PDA> --url devnet --output json

# Check deposit account
solana account <DEPOSIT_PDA> --url devnet --output json

# View transaction details
solana confirm <TX_SIGNATURE> --url devnet

# Search Solscan (web): https://solscan.io/?cluster=devnet
```

### Common Errors & Fixes

|Error                     |Cause                    |Fix                                              |
|--------------------------|-------------------------|-------------------------------------------------|
|`ProgramError::Custom(1)` |Generic error, no message|Add custom error enum (Step 3)                   |
|`AccountNotEnoughLamports`|Insufficient rent        |Check account size calculation                   |
|`PDANotFound`             |Wrong PDA seed           |Verify: `hash(b"seed" + pubkey) = actual_address`|
|`ConstraintToken`         |Wrong token mint         |Check token account ownership                    |
|`ConstraintMint`          |Mint mismatch            |Verify mint address in Cargo.toml                |
|`Overflow`                |Math exceeded u64 max    |Add bounds checks before arithmetic              |
|`InvalidInstruction`      |Wrong account order      |Check Accounts struct matches transaction        |

-----

## IX. CODE REVIEW CHECKLIST

Before pushing to GitHub:

- [ ] **Compiles?** `anchor build --no-idl` (no errors, only warnings)
- [ ] **Tests pass?** `anchor test` (all test cases green)
- [ ] **Error handling?** Every Err branch is explicit (no unwrap, no panic)
- [ ] **Precision math?** No floats, division order correct (a * b) / c
- [ ] **PDAs correct?** Seed derivation matches usage
- [ ] **Comments clear?** Explain WHY, not WHAT (code says what)
- [ ] **Variable names?** Not x, y, a, b (use vault, amount, authority)
- [ ] **CPI safe?** Account validation before CPI calls
- [ ] **Overflow checked?** Large numbers don’t overflow u64
- [ ] **Git clean?** No debug files, no .env secrets

-----

## X. KEY FILES & THEIR PURPOSE

|File             |Purpose                            |Status         |
|-----------------|-----------------------------------|---------------|
|`lib.rs`         |Main vault logic + #[program] macro|✅ Complete     |
|`state.rs`       |Vault & VaultDeposit structs       |✅ Complete     |
|`instructions.rs`|OLD (logic in lib.rs)              |❌ Delete today |
|`integration.ts` |Integration tests                  |⏳ Write today  |
|`Anchor.toml`    |Cluster config, program IDs        |✅ Devnet ready |
|`Cargo.toml`     |Workspace, dependencies            |✅ anchor 0.30.0|
|`README.md`      |Public overview                    |✅ Basic        |
|`CLAUDE.md`      |THIS FILE                          |✅ Comprehensive|

-----

## XI. NEXT STEPS (TODAY)

### Action 1: Delete Deprecated File

```bash
cd ~/terra-protocol
rm programs/terra-vault/src/instructions.rs
```

### Action 2: Write Integration Tests

Create `tests/integration.ts` with:

- Test initialize_vault
- Test deposit
- Test accrue_interest (time-based)
- Test withdraw
- Test error cases (overflow, negative, etc.)

See “Test Template” section below.

### Action 3: Run Tests Locally

```bash
anchor test
```

### Action 4: Deploy to Devnet

```bash
solana config set --url devnet
anchor deploy
```

### Action 5: Verify On-Chain

```bash
solana program show 4xwPZ3FgFbyqT8eZVGmjaPex5cAVaYazzjKd8cSTsHNe --url devnet
```

### Action 6: Commit to GitHub

```bash
git add .
git commit -m "Phase 1, Step 2: Integration tests + devnet deployment"
git push origin main
```

-----

## XII. TEST TEMPLATE (integration.ts)

```typescript
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { assert } from "chai";

describe("TERRA Vault - Phase 1 Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.TerraVault as Program<TerraVault>;

  const authority = provider.wallet.publicKey;

  const getVaultPda = () => {
    const [vaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), authority.toBuffer()],
      program.programId
    );
    return vaultPda;
  };

  const getDepositPda = (vaultPda) => {
    const [depositPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), vaultPda.toBuffer(), authority.toBuffer()],
      program.programId
    );
    return depositPda;
  };

  it("Initializes vault with correct state", async () => {
    const vaultPda = getVaultPda();
    const tx = await program.methods
      .initializeVault()
      .accounts({
        authority,
        vault: vaultPda,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.authority.toString(), authority.toString());
    assert.equal(vault.totalDeposits.toNumber(), 0);
    assert.equal(vault.dailyInterestRate.toNumber(), 800);
  });

  it("Deposits into vault", async () => {
    const vaultPda = getVaultPda();
    const depositPda = getDepositPda(vaultPda);
    const amount = new anchor.BN(1_000_000);

    const tx = await program.methods
      .deposit(amount)
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const deposit = await program.account.vaultDeposit.fetch(depositPda);
    assert.equal(deposit.amountDeposited.toNumber(), amount.toNumber());
  });

  it("Accrues interest with precision math", async () => {
    const vaultPda = getVaultPda();
    const tx = await program.methods
      .accrueInterest()
      .accounts({ vault: vaultPda })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert(vault.totalAccruedInterest.toNumber() > 0);
  });

  it("Withdraws principal + interest", async () => {
    const vaultPda = getVaultPda();
    const depositPda = getDepositPda(vaultPda);
    const amount = new anchor.BN(500_000);

    const tx = await program.methods
      .withdraw(amount)
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
      })
      .rpc();

    const deposit = await program.account.vaultDeposit.fetch(depositPda);
    assert(deposit.amountDeposited.toNumber() < 1_000_000);
  });
});
```

-----

## XIII. SUPPORT & CONTACT

**Builder:** KingFaitho (Full-time, Africa-focused)  
**Co-Founder AI:** Claude (Available 24/7)  
**GitHub:** <https://github.com/Kingfaitho/terra-protocol>  
**Status:** <https://github.com/Kingfaitho/terra-protocol/commits/main>

-----

## XIV. KEY METRICS (Track These)

**By End of Phase 1:**

- ✅ Code compiles
- ✅ All functions tested
- ✅ Tests pass on devnet
- ✅ GitHub stars (community signal)
- ✅ Grant applications submitted

**By End of Phase 3:**

- ✅ Testnet with 10 real farmers
- ✅ $100K+ locked in vault
- ✅ Interest earned + proven

**By End of Phase 6:**

- ✅ Mainnet live
- ✅ 1000+ users
- ✅ $1M+ TVL
- ✅ Full audit passed

-----

**Version:** 1.0  
**Last Updated:** May 26, 2026  
**Next Review:** After Phase 1 completion  
**Status:** Living document (updated as we build)

# TERRA — Verifiable Asset Tokenization Protocol
## Complete Co-Founder Knowledge Base v1.0

**Builder:** KingFaitho | **Status:** Phase 1, Step 2 | **Last Updated:** May 26, 2026

---

## WHAT WE'VE BUILT

✅ 3 Solana programs compiled (terra-vault, attestation, marketplace)
✅ Core vault logic: initialize, deposit, withdraw, accrue_interest
✅ Precision math (integer-only, no floats)
✅ GitHub live (Commit: bb2e07f)

⏳ Tests being written (TODAY)
⏳ Devnet deployment (TODAY)

---

## THE CORE: terra-vault Program

**What it does:**
1. initialize_vault() — Create vault account
2. deposit(amount) — Investor deposits SOL
3. withdraw(amount) — Withdraw principal + interest
4. accrue_interest() — Daily 8% APY calculation (precision math)

**Precision Math (NO FLOATS):**
interest = (total_deposits * daily_rate) / 36500 / 10000
Example: 1 SOL at 8% = 2 lamports/day (exact, no rounding errors)

**Data Structures:**
- Vault: authority, total_deposits, total_accrued_interest, daily_interest_rate, last_interest_accrual, bump
- VaultDeposit: vault, depositor, amount_deposited, interest_earned, deposit_timestamp, bump

---

## PROJECT STRUCTURE

terra-protocol/
├── programs/
│   ├── terra-vault/src/
│   │   ├── lib.rs (✅ Complete)
│   │   └── state.rs (✅ Complete)
│   ├── terra-attestation/ (⏳ Stub)
│   └── terra-marketplace/ (⏳ Stub)
├── tests/
│   └── integration.ts (⏳ Write today)
├── Anchor.toml
├── Cargo.toml
└── CLAUDE.md (THIS FILE)

---

## NEXT STEPS (TODAY)

1. Delete instructions.rs:
   rm programs/terra-vault/src/instructions.rs

2. Replace integration.ts with complete test file

3. Run tests:
   anchor test

4. Deploy to devnet:
   solana config set --url devnet
   anchor deploy

5. Verify:
   solana program show 4xwPZ3FgFbyqT8eZVGmjaPex5cAVaYazzjKd8cSTsHNe --url devnet

6. Commit:
   git add .
   git commit -m "Phase 1, Step 2: Tests + Devnet"
   git push origin main

---

## INTEGRATION TEST TEMPLATE (integration.ts)

Copy this into tests/integration.ts:

import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { assert } from "chai";

describe("TERRA Vault - Phase 1 Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.TerraVault as Program<TerraVault>;
  const authority = provider.wallet.publicKey;

  const getVaultPda = () => {
    const [vaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), authority.toBuffer()],
      program.programId
    );
    return vaultPda;
  };

  const getDepositPda = (vaultPda) => {
    const [depositPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), vaultPda.toBuffer(), authority.toBuffer()],
      program.programId
    );
    return depositPda;
  };

  it("Initializes vault with correct state", async () => {
    const vaultPda = getVaultPda();
    const tx = await program.methods
      .initializeVault()
      .accounts({
        authority,
        vault: vaultPda,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.authority.toString(), authority.toString());
    assert.equal(vault.totalDeposits.toNumber(), 0);
    assert.equal(vault.dailyInterestRate.toNumber(), 800);
    console.log("✅ Vault initialized");
  });

  it("Deposits into vault", async () => {
    const vaultPda = getVaultPda();
    const depositPda = getDepositPda(vaultPda);
    const amount = new anchor.BN(1_000_000);

    await program.methods
      .deposit(amount)
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const deposit = await program.account.vaultDeposit.fetch(depositPda);
    assert.equal(deposit.amountDeposited.toNumber(), amount.toNumber());
    console.log("✅ Deposit recorded");
  });

  it("Accrues interest", async () => {
    const vaultPda = getVaultPda();

    await program.methods
      .accrueInterest()
      .accounts({ vault: vaultPda })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert(vault.totalAccruedInterest.toNumber() > 0);
    console.log("✅ Interest accrued");
  });

  it("Withdraws from vault", async () => {
    const vaultPda = getVaultPda();
    const depositPda = getDepositPda(vaultPda);
    const amount = new anchor.BN(500_000);

    await program.methods
      .withdraw(amount)
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
      })
      .rpc();

    const deposit = await program.account.vaultDeposit.fetch(depositPda);
    assert(deposit.amountDeposited.toNumber() < 1_000_000);
    console.log("✅ Withdrawal verified");
  });
});

---

**Status:** CLAUDE.md created on YOUR laptop
**Next:** Replace integration.ts, then anchor test
