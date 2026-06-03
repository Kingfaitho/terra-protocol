import { startAnchor, BankrunProvider } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { TerraAttestation } from "../target/types/terra_attestation";
import { assert } from "chai";

const ATTESTATION_IDL = require("../target/idl/terra_attestation.json");

const IDL = require("../target/idl/terra_vault.json");

// Interest math mirrors lib.rs exactly — single source of truth for the test oracle
const DAILY_RATE = 800n; // 8% APY in basis points
function expectedDailyInterest(deposits: bigint): bigint {
  return (deposits * DAILY_RATE) / 36500n / 10000n;
}

describe("TERRA Vault - Interest Accrual (Bankrun + clock warp)", () => {
  let context: Awaited<ReturnType<typeof startAnchor>>;
  let provider: BankrunProvider;
  let program: Program<TerraVault>;
  let authority: anchor.web3.PublicKey;
  let vaultPda: anchor.web3.PublicKey;
  let depositPda: anchor.web3.PublicKey;

  const DEPOSIT_AMOUNT = 1_000_000n; // 0.001 SOL

  before(async () => {
    // startAnchor reads Anchor.toml, finds the .so, deploys it into an in-process validator
    context = await startAnchor(".", [], []);
    provider = new BankrunProvider(context);
    program = new Program<TerraVault>(IDL, provider);
    authority = provider.wallet.publicKey;

    [vaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), authority.toBuffer()],
      program.programId
    );
    [depositPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), vaultPda.toBuffer(), authority.toBuffer()],
      program.programId
    );
  });

  it("Initializes vault", async () => {
    await program.methods
      .initializeVault()
      .accounts({
        authority,
        vault: vaultPda,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.totalDeposits.toNumber(), 0);
    assert.equal(vault.dailyInterestRate.toNumber(), 800);
    console.log("  Vault initialized at:", vaultPda.toString());
  });

  it("Deposits 0.001 SOL into vault", async () => {
    await program.methods
      .deposit(new anchor.BN(DEPOSIT_AMOUNT.toString()))
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.totalDeposits.toString(), DEPOSIT_AMOUNT.toString());
    console.log(`  Deposited: ${DEPOSIT_AMOUNT} lamports`);
  });

  it("Warps clock +25 hours and accrues interest with correct precision math", async () => {
    // Read current on-chain clock before warp
    const clockBefore = await context.banksClient.getClock();

    // Warp 25 hours (well past the 24-hour gate)
    const WARP_SECONDS = 25n * 3600n;
    context.setClock(
      new Clock(
        clockBefore.slot,
        clockBefore.epochStartTimestamp,
        clockBefore.epoch,
        clockBefore.leaderScheduleEpoch,
        clockBefore.unixTimestamp + WARP_SECONDS
      )
    );

    // Accrue interest — no gate set, pass vaultPda as sentinel asset (ignored by program)
    await program.methods
      .accrueInterest()
      .accounts({ vault: vaultPda, asset: vaultPda })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    const accrued = BigInt(vault.totalAccruedInterest.toString());
    const expected = expectedDailyInterest(DEPOSIT_AMOUNT);

    assert.equal(
      accrued,
      expected,
      `Interest mismatch: got ${accrued}, expected ${expected}`
    );

    console.log(`  Clock warped +${WARP_SECONDS / 3600n}h`);
    console.log(`  Interest accrued: ${accrued} lamports (expected: ${expected})`);
    console.log(`  Formula check: (${DEPOSIT_AMOUNT} × ${DAILY_RATE}) / 36500 / 10000 = ${expected}`);
  });

  it("Rejects second accrual within the same 24-hour window", async () => {
    // No clock warp — still within the new 24h window
    try {
      await program.methods
        .accrueInterest()
        .accounts({ vault: vaultPda, asset: vaultPda })
        .rpc();
      assert.fail("Expected AccrualTooSoon error");
    } catch (err: any) {
      assert.include(err.toString(), "AccrualTooSoon");
      console.log("  Double-accrual correctly rejected (AccrualTooSoon / 6001)");
    }
  });

  it("Accrues again after a second clock warp (+25 more hours)", async () => {
    const clockBefore = await context.banksClient.getClock();
    const WARP_SECONDS = 25n * 3600n;

    context.setClock(
      new Clock(
        clockBefore.slot,
        clockBefore.epochStartTimestamp,
        clockBefore.epoch,
        clockBefore.leaderScheduleEpoch,
        clockBefore.unixTimestamp + WARP_SECONDS
      )
    );

    await program.methods
      .accrueInterest()
      .accounts({ vault: vaultPda, asset: vaultPda })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    const totalAccrued = BigInt(vault.totalAccruedInterest.toString());
    const expectedPerDay = expectedDailyInterest(DEPOSIT_AMOUNT);

    // Two full days accrued
    assert.equal(totalAccrued, expectedPerDay * 2n);
    console.log(`  Day 2 interest: total accrued = ${totalAccrued} lamports (${expectedPerDay * 2n} expected)`);
  });
});

// ─── Attestation Gate (cross-program, bankrun) ────────────────────────────────

describe("TERRA Vault - Attestation Gate (cross-program, Bankrun)", () => {
  // Fresh context for gate tests — both programs deployed via Anchor.toml
  let context: Awaited<ReturnType<typeof startAnchor>>;
  let provider: BankrunProvider;
  let vaultProgram: Program<TerraVault>;
  let attestProgram: Program<TerraAttestation>;
  let authority: anchor.web3.PublicKey;

  const agentBKp = anchor.web3.Keypair.generate();
  const agentCKp = anchor.web3.Keypair.generate();

  const STAKE = new anchor.BN(1_000_000);
  const DEPOSIT = new anchor.BN(1_000_000);
  const LOCATION_HASH = Array(32).fill(3);
  const ASSET_HASH = Array(32).fill(4);

  // ── PDA helpers ─────────────────────────────────────────────────────────────
  const vaultPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), auth.toBuffer()],
      vaultProgram.programId
    )[0];

  const depositPda = (vault: anchor.web3.PublicKey, dep: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), vault.toBuffer(), dep.toBuffer()],
      vaultProgram.programId
    )[0];

  const agentPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("agent"), auth.toBuffer()],
      attestProgram.programId
    )[0];

  const assetPda = (auth: anchor.web3.PublicKey, hash: number[]) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("asset"), auth.toBuffer(), Buffer.from(hash)],
      attestProgram.programId
    )[0];

  const attestationPda = (asset: anchor.web3.PublicKey, agent: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("attestation"), asset.toBuffer(), agent.toBuffer()],
      attestProgram.programId
    )[0];

  before(async () => {
    context = await startAnchor(".", [], []);
    provider = new BankrunProvider(context);
    vaultProgram = new Program<TerraVault>(IDL, provider);
    attestProgram = new Program<TerraAttestation>(ATTESTATION_IDL, provider);
    authority = provider.wallet.publicKey;

    // Fund agents B and C via SystemProgram transfer from the funded payer
    // (BankrunConnectionProxy doesn't expose requestAirdrop)
    for (const kp of [agentBKp, agentCKp]) {
      const tx = new anchor.web3.Transaction().add(
        anchor.web3.SystemProgram.transfer({
          fromPubkey: authority,
          toPubkey: kp.publicKey,
          lamports: 2_000_000_000,
        })
      );
      await provider.sendAndConfirm(tx);
    }
  });

  // ── Setup: vault + asset + 3-agent attestation ─────────────────────────────

  it("Sets up: vault initialized, 3 agents registered, asset attested to Verified", async () => {
    const vault = vaultPda(authority);
    const asset = assetPda(authority, ASSET_HASH);

    // Initialize vault
    await vaultProgram.methods
      .initializeVault()
      .accounts({ authority, vault, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // Deposit so interest math is non-zero later
    await vaultProgram.methods
      .deposit(DEPOSIT)
      .accounts({
        vault,
        vaultDeposit: depositPda(vault, authority),
        depositor: authority,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    // Register 3 agents
    for (const [kp, isMain] of [[null, true], [agentBKp, false], [agentCKp, false]] as [anchor.web3.Keypair | null, boolean][]) {
      const auth = kp ? kp.publicKey : authority;
      const agent = agentPda(auth);
      const tx = attestProgram.methods
        .registerAgent(STAKE)
        .accounts({ authority: auth, agent, systemProgram: anchor.web3.SystemProgram.programId });
      if (kp) tx.signers([kp]);
      await tx.rpc();
    }

    // Register asset (required_attestations = 3)
    await attestProgram.methods
      .registerAsset({ crop: {} }, new anchor.BN(500_000), LOCATION_HASH, ASSET_HASH, 3)
      .accounts({ authority, asset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // 3 agents attest → Verified
    for (const [kp, evidenceByte] of [
      [null, 0xaa], [agentBKp, 0xbb], [agentCKp, 0xcc],
    ] as [anchor.web3.Keypair | null, number][]) {
      const agentAuth = kp ? kp.publicKey : authority;
      const evidence = Array(32).fill(evidenceByte);
      const tx = attestProgram.methods
        .attestAsset(evidence)
        .accounts({
          agentAuthority: agentAuth,
          agent: agentPda(agentAuth),
          asset,
          attestation: attestationPda(asset, agentAuth),
          systemProgram: anchor.web3.SystemProgram.programId,
        });
      if (kp) tx.signers([kp]);
      await tx.rpc();
    }

    const assetState = await attestProgram.account.asset.fetch(asset);
    assert.deepEqual(assetState.status, { verified: {} });
    console.log(`  Asset status: Verified (3/3 attestations)`);
    console.log(`  Vault: ${vault.toString()}`);
    console.log(`  Asset: ${asset.toString()}`);
  });

  it("Rejects set_asset_gate when asset is Pending (not enough attestations)", async () => {
    // Register a fresh unverified asset with a different hash
    const pendingHash = Array(32).fill(99);
    const pendingAsset = assetPda(authority, pendingHash);

    await attestProgram.methods
      .registerAsset({ crop: {} }, new anchor.BN(100), LOCATION_HASH, pendingHash, 3)
      .accounts({ authority, asset: pendingAsset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // Only 0 attestations — still Pending
    try {
      await vaultProgram.methods
        .setAssetGate()
        .accounts({ authority, vault: vaultPda(authority), asset: pendingAsset })
        .rpc();
      assert.fail("Should have thrown AssetNotVerified");
    } catch (err: any) {
      assert.include(JSON.stringify(err), "AssetNotVerified");
      console.log("  Pending asset correctly rejected by set_asset_gate (6005)");
    }
  });

  it("Sets asset gate on vault — links Verified asset", async () => {
    const vault = vaultPda(authority);
    const asset = assetPda(authority, ASSET_HASH);

    await vaultProgram.methods
      .setAssetGate()
      .accounts({ authority, vault, asset })
      .rpc();

    const vaultState = await vaultProgram.account.vault.fetch(vault);
    assert.isNotNull(vaultState.linkedAsset);
    assert.equal(vaultState.linkedAsset!.toString(), asset.toString());
    console.log(`  Vault gate set → linked asset: ${asset.toString()}`);
  });

  it("Warps clock +25h — gated accrue_interest passes with Verified asset", async () => {
    const vault = vaultPda(authority);
    const asset = assetPda(authority, ASSET_HASH);

    const clockBefore = await context.banksClient.getClock();
    context.setClock(new Clock(
      clockBefore.slot,
      clockBefore.epochStartTimestamp,
      clockBefore.epoch,
      clockBefore.leaderScheduleEpoch,
      clockBefore.unixTimestamp + BigInt(25 * 3600),
    ));

    // accrue_interest WITH the verified asset account
    await vaultProgram.methods
      .accrueInterest()
      .accounts({ vault, asset })
      .rpc();

    const vaultState = await vaultProgram.account.vault.fetch(vault);
    const accrued = BigInt(vaultState.totalAccruedInterest.toString());
    const expected = expectedDailyInterest(BigInt(DEPOSIT.toString()));
    assert.equal(accrued, expected);
    console.log(`  Gated accrue succeeded: ${accrued} lamports interest (verified asset confirmed)`);
  });

  it("Rejects accrue_interest with a wrong (non-matching) asset account", async () => {
    const vault = vaultPda(authority);
    // Warp another 25h so the time gate passes
    const clockBefore = await context.banksClient.getClock();
    context.setClock(new Clock(
      clockBefore.slot, clockBefore.epochStartTimestamp,
      clockBefore.epoch, clockBefore.leaderScheduleEpoch,
      clockBefore.unixTimestamp + BigInt(25 * 3600),
    ));

    // vault PDA itself as wrong asset — key mismatch → InvalidAssetAccount
    try {
      await vaultProgram.methods
        .accrueInterest()
        .accounts({ vault, asset: vault })
        .rpc();
      assert.fail("Should have thrown InvalidAssetAccount");
    } catch (err: any) {
      assert.include(err.toString(), "InvalidAssetAccount");
      console.log("  Wrong asset account correctly rejected (InvalidAssetAccount 6006)");
    }
  });
});
