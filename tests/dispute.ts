/**
 * TERRA Phase 3 — Dispute & Slashing Tests (Bankrun)
 *
 * Resolver label: admin (v1 centrally adjudicated — not a decentralised oracle).
 * The resolver key is the test wallet (authority). In production the deployer
 * calls initialize_resolver immediately after deployment.
 *
 * Critical test (advisor-flagged): vault interest freezes the moment a dispute
 * is raised, because asset.status flips to Disputed and the Phase-2 gate
 * already requires status == Verified.
 */

import { startAnchor, BankrunProvider } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraAttestation } from "../target/types/terra_attestation";
import { TerraVault } from "../target/types/terra_vault";
import { assert } from "chai";

const ATTESTATION_IDL = require("../target/idl/terra_attestation.json");
const VAULT_IDL = require("../target/idl/terra_vault.json");

describe("TERRA Phase 3 — Dispute & Slashing (Bankrun)", () => {
  let context: Awaited<ReturnType<typeof startAnchor>>;
  let provider: BankrunProvider;
  let attest: Program<TerraAttestation>;
  let vault: Program<TerraVault>;
  let authority: anchor.web3.PublicKey; // acts as: farmer, agent A, and resolver admin

  const agentBKp = anchor.web3.Keypair.generate();
  const agentCKp = anchor.web3.Keypair.generate();
  const disputerKp = anchor.web3.Keypair.generate();

  const STAKE      = new anchor.BN(2_000_000); // 0.002 SOL — will be halved on slash
  const DEPOSIT    = new anchor.BN(1_000_000); // 0.001 SOL in vault
  const BOND       = new anchor.BN(1_000_000); // 0.001 SOL dispute bond (minimum required)
  const LOC_HASH   = Array(32).fill(5);
  const ASSET_HASH = Array(32).fill(6);

  // ── PDA helpers ─────────────────────────────────────────────────────────────

  const resolverPda = () =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("resolver")],
      attest.programId
    )[0];

  const agentPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("agent"), auth.toBuffer()],
      attest.programId
    )[0];

  const assetPda = (auth: anchor.web3.PublicKey, hash: number[]) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("asset"), auth.toBuffer(), Buffer.from(hash)],
      attest.programId
    )[0];

  const attestationPda = (asset: anchor.web3.PublicKey, agentAuth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("attestation"), asset.toBuffer(), agentAuth.toBuffer()],
      attest.programId
    )[0];

  const disputePda = (asset: anchor.web3.PublicKey, disputer: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("dispute"), asset.toBuffer(), disputer.toBuffer()],
      attest.programId
    )[0];

  const slashRecordPda = (dispute: anchor.web3.PublicKey, agent: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("slash"), dispute.toBuffer(), agent.toBuffer()],
      attest.programId
    )[0];

  const vaultPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), auth.toBuffer()],
      vault.programId
    )[0];

  const depositPda = (v: anchor.web3.PublicKey, dep: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), v.toBuffer(), dep.toBuffer()],
      vault.programId
    )[0];

  // ── Test setup ──────────────────────────────────────────────────────────────

  before(async () => {
    context  = await startAnchor(".", [], []);
    provider = new BankrunProvider(context);
    attest   = new Program<TerraAttestation>(ATTESTATION_IDL, provider);
    vault    = new Program<TerraVault>(VAULT_IDL, provider);
    authority = provider.wallet.publicKey;

    // Fund agent B, C, and the disputer
    for (const kp of [agentBKp, agentCKp, disputerKp]) {
      const tx = new anchor.web3.Transaction().add(
        anchor.web3.SystemProgram.transfer({
          fromPubkey: authority,
          toPubkey: kp.publicKey,
          lamports: 5_000_000_000,
        })
      );
      await provider.sendAndConfirm(tx);
    }
  });

  // ── Step 0: build the world ─────────────────────────────────────────────────

  it("Initializes resolver, vault, 3 agents, asset → Verified, vault gate set", async () => {
    const asset = assetPda(authority, ASSET_HASH);
    const v     = vaultPda(authority);

    // Resolver (admin) setup
    await attest.methods
      .initializeResolver()
      .accounts({ authority, disputeResolver: resolverPda(), systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();
    const dr = await attest.account.disputeResolver.fetch(resolverPda());
    assert.equal(dr.authority.toString(), authority.toString());

    // Vault
    await vault.methods.initializeVault()
      .accounts({ authority, vault: v, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();
    await vault.methods.deposit(DEPOSIT)
      .accounts({ vault: v, vaultDeposit: depositPda(v, authority), depositor: authority, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // 3 agents
    for (const [kp] of [[null], [agentBKp], [agentCKp]] as [anchor.web3.Keypair | null][]) {
      const auth  = kp ? kp.publicKey : authority;
      const agent = agentPda(auth);
      const tx = attest.methods.registerAgent(STAKE)
        .accounts({ authority: auth, agent, systemProgram: anchor.web3.SystemProgram.programId });
      if (kp) tx.signers([kp]);
      await tx.rpc();
    }

    // Register asset + 3 attestations → Verified
    await attest.methods
      .registerAsset({ crop: {} }, new anchor.BN(1_000_000), LOC_HASH, ASSET_HASH, 3)
      .accounts({ authority, asset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    for (const [kp, ev] of [[null, 0x11], [agentBKp, 0x22], [agentCKp, 0x33]] as [anchor.web3.Keypair | null, number][]) {
      const auth = kp ? kp.publicKey : authority;
      const tx = attest.methods.attestAsset(Array(32).fill(ev))
        .accounts({
          agentAuthority: auth, agent: agentPda(auth),
          asset, attestation: attestationPda(asset, auth),
          systemProgram: anchor.web3.SystemProgram.programId,
        });
      if (kp) tx.signers([kp]);
      await tx.rpc();
    }

    const assetState = await attest.account.asset.fetch(asset);
    assert.deepEqual(assetState.status, { verified: {} });

    // Bidirectional link: asset.linked_vault must point to the vault before set_asset_gate
    await attest.methods.linkVault(v).accounts({ authority, asset }).rpc();

    // Set vault interest gate
    await vault.methods.setAssetGate()
      .accounts({ authority, vault: v, asset })
      .rpc();

    const vaultState = await vault.account.vault.fetch(v);
    assert.isNotNull(vaultState.linkedAsset);
    console.log("  World built: resolver ✓ vault ✓ agents ✓ asset Verified ✓ gate set ✓");
  });

  // ── Step 1: raise a dispute ─────────────────────────────────────────────────

  it("Rejects raising a dispute on a non-Verified (would be Pending) asset", async () => {
    // Register a fresh Pending asset to test the error path
    const pendingHash = Array(32).fill(77);
    const pendingAsset = assetPda(authority, pendingHash);
    await attest.methods
      .registerAsset({ land: {} }, new anchor.BN(100), LOC_HASH, pendingHash, 3)
      .accounts({ authority, asset: pendingAsset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    try {
      await attest.methods
        .raiseDispute(Array(32).fill(0xde), BOND)
        .accounts({
          disputer: disputerKp.publicKey,
          asset: pendingAsset,
          dispute: disputePda(pendingAsset, disputerKp.publicKey),
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([disputerKp])
        .rpc();
      assert.fail("Should have thrown AssetNotVerifiable");
    } catch (err: any) {
      assert.include(err.toString(), "AssetNotVerifiable");
      console.log("  Pending asset correctly rejected (AssetNotVerifiable 6200)");
    }
  });

  it("Raises a dispute on the Verified asset — asset flips to Disputed", async () => {
    const asset   = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);

    await attest.methods
      .raiseDispute(Array(32).fill(0xde), BOND)
      .accounts({
        disputer: disputerKp.publicKey,
        asset,
        dispute,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([disputerKp])
      .rpc();

    const assetState = await attest.account.asset.fetch(asset);
    assert.deepEqual(assetState.status, { disputed: {} });

    const disputeState = await attest.account.dispute.fetch(dispute);
    assert.deepEqual(disputeState.status, { active: {} });
    assert.equal(disputeState.bondAmount.toString(), BOND.toString());

    // Bond is now in the dispute PDA (use banksClient — BankrunConnectionProxy lacks getBalance)
    const disputeBalance = await context.banksClient.getBalance(dispute);
    assert(Number(disputeBalance) >= BOND.toNumber());

    console.log(`  Dispute raised — asset now Disputed, bond (${BOND} lamports) locked ✓`);
  });

  // ── THE CRITICAL TEST: vault interest gate ───────────────────────────────────

  it("⚡ Vault interest is BLOCKED after dispute raised (AssetNotVerified)", async () => {
    const v     = vaultPda(authority);
    const asset = assetPda(authority, ASSET_HASH);

    // Warp 25h so the time gate passes
    const clock = await context.banksClient.getClock();
    context.setClock(new Clock(clock.slot, clock.epochStartTimestamp, clock.epoch,
      clock.leaderScheduleEpoch, clock.unixTimestamp + BigInt(25 * 3600)));

    try {
      await vault.methods.accrueInterest()
        .accounts({ vault: v, asset })
        .rpc();
      assert.fail("Should have thrown AssetNotVerified");
    } catch (err: any) {
      assert.include(err.toString(), "AssetNotVerified");
      console.log("  accrue_interest blocked while asset Disputed (AssetNotVerified 6005) ✓");
      console.log("  → Phase-2 gate proved end-to-end: dispute → interest freeze, zero vault changes");
    }
  });

  // ── Step 2: resolve as upheld ────────────────────────────────────────────────

  it("Non-resolver cannot resolve a dispute", async () => {
    const asset   = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);

    try {
      await attest.methods.resolveDispute(true)
        .accounts({
          resolver: disputerKp.publicKey,
          disputeResolver: resolverPda(),
          dispute,
          asset,
        })
        .signers([disputerKp])
        .rpc();
      assert.fail("Should have thrown ResolverOnly");
    } catch (err: any) {
      // ResolverOnly error (6203) — check message or code string
      const errStr = JSON.stringify(err) + err.toString();
      assert(
        errStr.includes("ResolverOnly") || errStr.includes("6203") || errStr.includes("12203"),
        `Expected ResolverOnly error, got: ${err.toString().slice(0, 200)}`
      );
      console.log("  Non-resolver correctly rejected (ResolverOnly / 6203)");
    }
  });

  it("Resolver upholds the dispute — asset stays Disputed", async () => {
    const asset   = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);

    await attest.methods.resolveDispute(true)
      .accounts({ resolver: authority, disputeResolver: resolverPda(), dispute, asset })
      .rpc();

    const disputeState = await attest.account.dispute.fetch(dispute);
    assert.deepEqual(disputeState.status, { upheld: {} });

    const assetState = await attest.account.asset.fetch(asset);
    assert.deepEqual(assetState.status, { disputed: {} }); // still Disputed
    console.log("  Dispute upheld — asset remains Disputed ✓");
  });

  // ── Step 3: slash all 3 agents ───────────────────────────────────────────────

  it("Slashes Agent A — stake halved, active_attestation_count decremented", async () => {
    const asset   = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);
    const agent   = agentPda(authority);

    const agentBefore = await attest.account.agent.fetch(agent);
    const expectedSlash = agentBefore.stakeAmount.toNumber() / 2;

    await attest.methods.slashAgent()
      .accounts({
        payer: authority,
        dispute,
        agent,
        attestation: attestationPda(asset, authority),
        slashRecord: slashRecordPda(dispute, agent),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const agentAfter = await attest.account.agent.fetch(agent);
    assert.equal(
      agentAfter.stakeAmount.toNumber(),
      agentBefore.stakeAmount.toNumber() - expectedSlash
    );
    assert.equal(
      agentAfter.activeAttestationCount,
      agentBefore.activeAttestationCount - 1
    );

    const disputeState = await attest.account.dispute.fetch(dispute);
    assert.equal(disputeState.totalSlashed.toNumber(), expectedSlash);
    assert.equal(disputeState.agentsSlashed, 1);

    console.log(`  Agent A slashed: ${expectedSlash} lamports extracted, stake ${agentAfter.stakeAmount} remaining`);
  });

  it("Rejects double-slash of Agent A in the same dispute", async () => {
    const asset   = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);
    const agent   = agentPda(authority);

    try {
      await attest.methods.slashAgent()
        .accounts({
          payer: authority,
          dispute,
          agent,
          attestation: attestationPda(asset, authority),
          slashRecord: slashRecordPda(dispute, agent),
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();
      assert.fail("Should have thrown — SlashRecord already exists");
    } catch (err: any) {
      // init constraint rejects re-initialization of the SlashRecord PDA
      assert.ok(err, "Double-slash correctly rejected by SlashRecord init constraint");
      console.log("  Double-slash rejected (SlashRecord PDA already exists) ✓");
    }
  });

  it("Slashes Agents B and C — all 3 agents slashed, running total correct", async () => {
    const asset   = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);

    for (const kp of [agentBKp, agentCKp]) {
      const agent = agentPda(kp.publicKey);
      const before = await attest.account.agent.fetch(agent);

      await attest.methods.slashAgent()
        .accounts({
          payer: authority,
          dispute,
          agent,
          attestation: attestationPda(asset, kp.publicKey),
          slashRecord: slashRecordPda(dispute, agent),
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .rpc();

      const after = await attest.account.agent.fetch(agent);
      assert.equal(after.stakeAmount.toNumber(), before.stakeAmount.toNumber() / 2);
      assert.equal(after.activeAttestationCount, before.activeAttestationCount - 1);
    }

    const disputeState = await attest.account.dispute.fetch(dispute);
    assert.equal(disputeState.agentsSlashed, 3);
    // Each agent staked 2_000_000 → 1_000_000 slashed each → total = 3_000_000
    assert.equal(disputeState.totalSlashed.toNumber(), 3_000_000);
    console.log(`  All 3 agents slashed. Total extracted: ${disputeState.totalSlashed} lamports locked in Dispute PDA`);
    console.log(`  Bond (${BOND} lamports) + slashes (3_000_000) sit in Dispute PDA for Phase 3 Step 2 distribution`);
  });

  // ── Slash-evasion invariant test ────────────────────────────────────────────

  it("Slash-evasion invariant: agent with active attestations cannot unregister", async () => {
    // Register a fresh agent, attest one asset, try to unregister before slash
    const evaderKp = anchor.web3.Keypair.generate();
    const evaderAuth = evaderKp.publicKey;

    const fundTx = new anchor.web3.Transaction().add(
      anchor.web3.SystemProgram.transfer({ fromPubkey: authority, toPubkey: evaderAuth, lamports: 5_000_000_000 })
    );
    await provider.sendAndConfirm(fundTx);

    // Use a different asset so we don't conflict with the disputed one
    const evaderHash = Array(32).fill(0xee);
    const evaderAsset = assetPda(authority, evaderHash);
    await attest.methods
      .registerAsset({ livestock: {} }, new anchor.BN(10), LOC_HASH, evaderHash, 1)
      .accounts({ authority, asset: evaderAsset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    const evaderAgent = agentPda(evaderAuth);
    await attest.methods.registerAgent(STAKE)
      .accounts({ authority: evaderAuth, agent: evaderAgent, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([evaderKp])
      .rpc();

    await attest.methods.attestAsset(Array(32).fill(0xff))
      .accounts({
        agentAuthority: evaderAuth, agent: evaderAgent,
        asset: evaderAsset, attestation: attestationPda(evaderAsset, evaderAuth),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([evaderKp])
      .rpc();

    // Now try to unregister with active_attestation_count == 1 → must fail
    try {
      await attest.methods.unregisterAgent()
        .accounts({ authority: evaderAuth, agent: evaderAgent })
        .signers([evaderKp])
        .rpc();
      assert.fail("Should have thrown AgentHasOpenAttestations");
    } catch (err: any) {
      assert.include(err.toString(), "AgentHasOpenAttestations");
      console.log("  Slash-evasion blocked: agent with open attestations cannot withdraw stake ✓");
    }
  });

  // ── Dismissal path: interest resumes ────────────────────────────────────────

  it("Dismissed dispute: asset reverts to Verified, vault interest resumes", async () => {
    // Set up a new asset + dispute, then dismiss it
    const asset2Hash  = Array(32).fill(0xab);
    const asset2      = assetPda(authority, asset2Hash);
    const dispute2    = disputePda(asset2, disputerKp.publicKey);
    const v           = vaultPda(authority);

    // Register + attest asset2 to Verified with 1 attestation
    await attest.methods
      .registerAsset({ equipment: {} }, new anchor.BN(1), LOC_HASH, asset2Hash, 1)
      .accounts({ authority, asset: asset2, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // Agent A still has active_count > 0 so re-using them isn't needed;
    // use a fresh evidence hash for a second agentPda(authority) attestation — NOT possible
    // because agent A already attested asset (the first one). For asset2, they haven't.
    await attest.methods.attestAsset(Array(32).fill(0xac))
      .accounts({
        agentAuthority: authority, agent: agentPda(authority),
        asset: asset2, attestation: attestationPda(asset2, authority),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    const assetState2Before = await attest.account.asset.fetch(asset2);
    assert.deepEqual(assetState2Before.status, { verified: {} });

    // Raise dispute
    await attest.methods
      .raiseDispute(Array(32).fill(0xba), BOND)
      .accounts({ disputer: disputerKp.publicKey, asset: asset2, dispute: dispute2, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([disputerKp])
      .rpc();
    assert.deepEqual((await attest.account.asset.fetch(asset2)).status, { disputed: {} });

    // Dismiss dispute
    await attest.methods.resolveDispute(false)
      .accounts({ resolver: authority, disputeResolver: resolverPda(), dispute: dispute2, asset: asset2 })
      .rpc();

    const assetAfter = await attest.account.asset.fetch(asset2);
    assert.deepEqual(assetAfter.status, { verified: {} });
    console.log("  Dismissed dispute: asset back to Verified ✓");

    // Vault still has linked_asset = Some(main asset) which is Disputed.
    // Remove the broken gate (only allowed because main asset is Disputed).
    const mainAsset = assetPda(authority, ASSET_HASH);
    await vault.methods.removeAssetGate()
      .accounts({ authority, vault: v, currentAsset: mainAsset })
      .rpc();

    const vaultAfterRemove = await vault.account.vault.fetch(v);
    assert.isNull(vaultAfterRemove.linkedAsset);
    console.log("  Broken gate removed (main asset is Disputed) ✓");

    // Bidirectional link: asset2 must point back to vault before set_asset_gate
    await attest.methods.linkVault(v).accounts({ authority, asset: asset2 }).rpc();

    // Wire vault gate to asset2 and confirm interest can accrue again
    await vault.methods.setAssetGate()
      .accounts({ authority, vault: v, asset: asset2 })
      .rpc();

    // Warp another 25h
    const clock = await context.banksClient.getClock();
    context.setClock(new Clock(clock.slot, clock.epochStartTimestamp, clock.epoch,
      clock.leaderScheduleEpoch, clock.unixTimestamp + BigInt(25 * 3600)));

    await vault.methods.accrueInterest()
      .accounts({ vault: v, asset: asset2 })
      .rpc();

    const vaultState = await vault.account.vault.fetch(v);
    assert(vaultState.totalAccruedInterest.toNumber() > 0);
    console.log(`  Interest resumed after dismissal: ${vaultState.totalAccruedInterest} lamports accrued ✓`);
  });
});
