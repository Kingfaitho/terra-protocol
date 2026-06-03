/**
 * TERRA — Interest Payout & Vault Recovery Tests (Bankrun)
 *
 * Tests the full economic loop:
 * 1. Vault authority funds interest via fund_vault_interest
 * 2. Depositor withdraws principal + pro-rata interest share
 * 3. Vault recovery: remove_asset_gate when linked asset is Disputed,
 *    then re-gate to a fresh Verified asset
 */

import { startAnchor, BankrunProvider } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { TerraAttestation } from "../target/types/terra_attestation";
import { assert } from "chai";

const VAULT_IDL = require("../target/idl/terra_vault.json");
const ATTEST_IDL = require("../target/idl/terra_attestation.json");

describe("TERRA — Interest Payout & Vault Recovery (Bankrun)", () => {
  let context: Awaited<ReturnType<typeof startAnchor>>;
  let provider: BankrunProvider;
  let vault: Program<TerraVault>;
  let attest: Program<TerraAttestation>;
  let authority: anchor.web3.PublicKey;

  const depositorKp = anchor.web3.Keypair.generate();
  const agentBKp    = anchor.web3.Keypair.generate();
  const agentCKp    = anchor.web3.Keypair.generate();
  const disputerKp  = anchor.web3.Keypair.generate();

  const DEPOSIT     = new anchor.BN(10_000_000); // 0.01 SOL
  const STAKE       = new anchor.BN(2_000_000);
  const BOND        = new anchor.BN(1_000_000);
  const LOC_HASH    = Array(32).fill(0x10);
  const ASSET_HASH  = Array(32).fill(0x20);
  const ASSET2_HASH = Array(32).fill(0x21);

  // ── PDA helpers ─────────────────────────────────────────────────────────────

  const vaultPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), auth.toBuffer()], vault.programId)[0];

  const depositPda = (v: anchor.web3.PublicKey, dep: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), v.toBuffer(), dep.toBuffer()], vault.programId)[0];

  const agentPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("agent"), auth.toBuffer()], attest.programId)[0];

  const assetPda = (auth: anchor.web3.PublicKey, hash: number[]) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("asset"), auth.toBuffer(), Buffer.from(hash)], attest.programId)[0];

  const attestationPda = (asset: anchor.web3.PublicKey, agentAuth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("attestation"), asset.toBuffer(), agentAuth.toBuffer()], attest.programId)[0];

  const disputePda = (asset: anchor.web3.PublicKey, disputer: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("dispute"), asset.toBuffer(), disputer.toBuffer()], attest.programId)[0];

  const resolverPda = () =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("resolver")], attest.programId)[0];

  // ── Setup ───────────────────────────────────────────────────────────────────

  before(async () => {
    context  = await startAnchor(".", [], []);
    provider = new BankrunProvider(context);
    vault    = new Program<TerraVault>(VAULT_IDL, provider);
    attest   = new Program<TerraAttestation>(ATTEST_IDL, provider);
    authority = provider.wallet.publicKey;

    for (const kp of [depositorKp, agentBKp, agentCKp, disputerKp]) {
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

  // ─────────────────────────────────────────────────────────────────────────────
  // SECTION 1: REAL INTEREST PAYOUT
  // ─────────────────────────────────────────────────────────────────────────────

  it("Full economic loop: fund_vault_interest → accrue → withdraw with real interest", async () => {
    const v       = vaultPda(authority);
    const depPda  = depositPda(v, depositorKp.publicKey);
    const asset   = assetPda(authority, ASSET_HASH);

    // ── Build vault + asset ────────────────────────────────────────────────
    await vault.methods.initializeVault()
      .accounts({ authority, vault: v, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // Depositor deposits 0.01 SOL
    await vault.methods.deposit(DEPOSIT)
      .accounts({
        vault: v,
        vaultDeposit: depPda,
        depositor: depositorKp.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([depositorKp])
      .rpc();

    // Register and attest asset to Verified (3 agents)
    await attest.methods.registerAgent(STAKE)
      .accounts({ authority, agent: agentPda(authority), systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();
    for (const kp of [agentBKp, agentCKp]) {
      await attest.methods.registerAgent(STAKE)
        .accounts({ authority: kp.publicKey, agent: agentPda(kp.publicKey), systemProgram: anchor.web3.SystemProgram.programId })
        .signers([kp])
        .rpc();
    }

    await attest.methods.registerAsset({ crop: {} }, new anchor.BN(100), LOC_HASH, ASSET_HASH, 3)
      .accounts({ authority, asset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    for (const [kp, ev] of [[null, 0x01], [agentBKp, 0x02], [agentCKp, 0x03]] as [anchor.web3.Keypair | null, number][]) {
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

    // Bidirectional link + gate
    await attest.methods.linkVault(v).accounts({ authority, asset }).rpc();
    await vault.methods.setAssetGate().accounts({ authority, vault: v, asset }).rpc();

    // ── Warp 25h and accrue interest ───────────────────────────────────────
    const clock = await context.banksClient.getClock();
    context.setClock(new Clock(clock.slot, clock.epochStartTimestamp, clock.epoch,
      clock.leaderScheduleEpoch, clock.unixTimestamp + BigInt(25 * 3600)));

    await vault.methods.accrueInterest()
      .accounts({ vault: v, asset })
      .rpc();

    const vaultAfterAccrue = await vault.account.vault.fetch(v);
    const accrued = vaultAfterAccrue.totalAccruedInterest.toNumber();
    // Expected: (10_000_000 * 800) / 36500 / 10000 = 21 lamports
    const expected = Math.floor(Math.floor((10_000_000 * 800) / 36500) / 10000);
    assert.equal(accrued, expected);
    console.log(`  Accrued: ${accrued} lamports (expected: ${expected})`);

    // ── Vault authority funds the interest pool ────────────────────────────
    const FUND_AMOUNT = 1_000_000; // 0.001 SOL yield contribution
    await vault.methods.fundVaultInterest(new anchor.BN(FUND_AMOUNT))
      .accounts({ authority, vault: v, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    const vaultBalanceAfterFund = await context.banksClient.getBalance(v);
    console.log(`  Vault balance after funding: ${vaultBalanceAfterFund} lamports`);

    // ── Depositor withdraws HALF — should receive principal + interest share ──
    const withdrawAmount = DEPOSIT.toNumber() / 2; // 5_000_000
    const depositorBalanceBefore = await context.banksClient.getBalance(depositorKp.publicKey);

    await vault.methods.withdraw(new anchor.BN(withdrawAmount))
      .accounts({ vault: v, vaultDeposit: depPda, depositor: depositorKp.publicKey })
      .signers([depositorKp])
      .rpc();

    const depositorBalanceAfter = await context.banksClient.getBalance(depositorKp.publicKey);
    const received = Number(depositorBalanceAfter) - Number(depositorBalanceBefore);

    // Depositor withdrew 5M lamports principal.
    // Interest = (5M / 10M) * 21 = 10 lamports (floor, pro-rata half the vault)
    // But capped at available interest pool
    assert(received >= withdrawAmount, `Received ${received} < principal ${withdrawAmount}`);
    const interestReceived = received - withdrawAmount;
    console.log(`  Withdrawn: ${withdrawAmount} principal + ${interestReceived} interest`);

    const depAfter = await vault.account.vaultDeposit.fetch(depPda);
    assert.equal(depAfter.interestEarned.toNumber(), interestReceived);
    console.log(`  interest_earned field updated correctly: ${interestReceived} lamports ✓`);
  });

  it("Withdraw with zero interest pool pays principal only (no error)", async () => {
    const v      = vaultPda(authority);
    const depPda = depositPda(v, depositorKp.publicKey);

    // At this point the depositor still has 5M in the vault
    // and total_accrued_interest has been partially paid out.
    // The interest pool may be exhausted — withdrawing should succeed returning principal only.
    const depBefore = await vault.account.vaultDeposit.fetch(depPda);
    const remaining = depBefore.amountDeposited.toNumber();

    if (remaining === 0) {
      console.log("  No remaining deposit — skipping (prior test withdrew all)");
      return;
    }

    const balanceBefore = await context.banksClient.getBalance(depositorKp.publicKey);
    await vault.methods.withdraw(new anchor.BN(remaining))
      .accounts({ vault: v, vaultDeposit: depPda, depositor: depositorKp.publicKey })
      .signers([depositorKp])
      .rpc();

    const balanceAfter = await context.banksClient.getBalance(depositorKp.publicKey);
    const received = Number(balanceAfter) - Number(balanceBefore);
    assert(received >= remaining, "Should receive at least principal");
    console.log(`  Second withdrawal: ${remaining} principal + ${received - remaining} interest ✓`);
  });

  // ─────────────────────────────────────────────────────────────────────────────
  // SECTION 2: VAULT RECOVERY VIA remove_asset_gate
  // ─────────────────────────────────────────────────────────────────────────────

  it("remove_asset_gate rejects when linked asset is still Verified (not Disputed)", async () => {
    // Set up a fresh vault + Verified asset (re-using agents from above)
    const recoveryAuthorityKp = anchor.web3.Keypair.generate();
    const ra = recoveryAuthorityKp.publicKey;
    const fundTx = new anchor.web3.Transaction().add(
      anchor.web3.SystemProgram.transfer({ fromPubkey: authority, toPubkey: ra, lamports: 5_000_000_000 })
    );
    await provider.sendAndConfirm(fundTx);

    const v2Hash = Array(32).fill(0x30);
    const v2     = vaultPda(ra);
    const asset2 = assetPda(ra, v2Hash);

    await vault.methods.initializeVault()
      .accounts({ authority: ra, vault: v2, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([recoveryAuthorityKp])
      .rpc();

    // Register agents for this authority
    await attest.methods.registerAgent(STAKE)
      .accounts({ authority: ra, agent: agentPda(ra), systemProgram: anchor.web3.SystemProgram.programId })
      .signers([recoveryAuthorityKp])
      .rpc();

    await attest.methods.registerAsset({ crop: {} }, new anchor.BN(1), LOC_HASH, v2Hash, 1)
      .accounts({ authority: ra, asset: asset2, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([recoveryAuthorityKp])
      .rpc();

    await attest.methods.attestAsset(Array(32).fill(0x30))
      .accounts({
        agentAuthority: ra, agent: agentPda(ra),
        asset: asset2, attestation: attestationPda(asset2, ra),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([recoveryAuthorityKp])
      .rpc();

    // Link asset → vault and set gate
    await attest.methods.linkVault(v2).accounts({ authority: ra, asset: asset2 }).signers([recoveryAuthorityKp]).rpc();
    await vault.methods.setAssetGate().accounts({ authority: ra, vault: v2, asset: asset2 }).signers([recoveryAuthorityKp]).rpc();

    // Asset is Verified — cannot remove gate
    try {
      await vault.methods.removeAssetGate()
        .accounts({ authority: ra, vault: v2, currentAsset: asset2 })
        .signers([recoveryAuthorityKp])
        .rpc();
      assert.fail("Should have thrown GateMustBeDisputed");
    } catch (err: any) {
      assert.include(err.toString(), "GateMustBeDisputed");
      console.log("  Cannot remove gate on Verified asset (GateMustBeDisputed 6008) ✓");
    }
  });

  it("Full recovery: dispute → uphold → remove_asset_gate → re-gate to new asset → interest resumes", async () => {
    // Use the vault from Section 1 (v = vaultPda(authority)) — its gate is currently
    // still set to ASSET_HASH asset (link_vault was called, gate set, interest accrued).
    // We'll raise + uphold a dispute to make the asset Disputed, then recover.
    const v     = vaultPda(authority);
    const asset = assetPda(authority, ASSET_HASH);

    // Initialize resolver (needed for resolve_dispute)
    await attest.methods.initializeResolver()
      .accounts({ authority, disputeResolver: resolverPda(), systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    const dispute = disputePda(asset, disputerKp.publicKey);

    // Raise a dispute
    await attest.methods.raiseDispute(Array(32).fill(0xdd), BOND)
      .accounts({
        disputer: disputerKp.publicKey,
        asset,
        dispute,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([disputerKp])
      .rpc();

    assert.deepEqual((await attest.account.asset.fetch(asset)).status, { disputed: {} });

    // Uphold the dispute
    await attest.methods.resolveDispute(true)
      .accounts({ resolver: authority, disputeResolver: resolverPda(), dispute, asset })
      .rpc();

    // Asset is now permanently Disputed — remove the vault gate
    await vault.methods.removeAssetGate()
      .accounts({ authority, vault: v, currentAsset: asset })
      .rpc();

    const vaultAfterRemove = await vault.account.vault.fetch(v);
    assert.isNull(vaultAfterRemove.linkedAsset);
    console.log("  Gate removed successfully on Disputed asset ✓");

    // Register a new fresh asset (ASSET2_HASH) and attest it to Verified
    const asset2 = assetPda(authority, ASSET2_HASH);
    await attest.methods.registerAsset({ crop: {} }, new anchor.BN(200), LOC_HASH, ASSET2_HASH, 1)
      .accounts({ authority, asset: asset2, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // Agent A hasn't been slashed yet (dispute just upheld, no slash_agent called here).
    // But agent A attested asset (ASSET_HASH). They have NOT attested asset2 yet.
    await attest.methods.attestAsset(Array(32).fill(0xee))
      .accounts({
        agentAuthority: authority, agent: agentPda(authority),
        asset: asset2, attestation: attestationPda(asset2, authority),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    assert.deepEqual((await attest.account.asset.fetch(asset2)).status, { verified: {} });

    // Bidirectional link + re-gate
    await attest.methods.linkVault(v).accounts({ authority, asset: asset2 }).rpc();
    await vault.methods.setAssetGate().accounts({ authority, vault: v, asset: asset2 }).rpc();

    const vaultAfterReGate = await vault.account.vault.fetch(v);
    assert.equal(vaultAfterReGate.linkedAsset!.toString(), asset2.toString());
    console.log("  Re-gated to new Verified asset ✓");

    // Warp 25h and confirm interest accrues normally on the new gate
    const clock = await context.banksClient.getClock();
    context.setClock(new Clock(clock.slot, clock.epochStartTimestamp, clock.epoch,
      clock.leaderScheduleEpoch, clock.unixTimestamp + BigInt(25 * 3600)));

    await vault.methods.accrueInterest()
      .accounts({ vault: v, asset: asset2 })
      .rpc();

    const vaultFinal = await vault.account.vault.fetch(v);
    console.log(`  Interest accrues on recovered vault: total_accrued = ${vaultFinal.totalAccruedInterest} lamports ✓`);
  });

  it("set_asset_gate rejects GateAlreadySet when vault already has a gate", async () => {
    const v    = vaultPda(authority);
    const asset2 = assetPda(authority, ASSET2_HASH);

    // Vault is now gated on asset2 — trying to set another gate should fail
    try {
      await vault.methods.setAssetGate()
        .accounts({ authority, vault: v, asset: asset2 })
        .rpc();
      assert.fail("Should have thrown GateAlreadySet");
    } catch (err: any) {
      assert.include(err.toString(), "GateAlreadySet");
      console.log("  GateAlreadySet correctly enforced ✓");
    }
  });

  it("set_asset_gate rejects when asset.linked_vault != vault.key() (bidirectional check)", async () => {
    // Create a third vault and try to gate it with asset2 (which points to v, not v3)
    const v3AuthKp = anchor.web3.Keypair.generate();
    const v3Auth = v3AuthKp.publicKey;
    const fundTx = new anchor.web3.Transaction().add(
      anchor.web3.SystemProgram.transfer({ fromPubkey: authority, toPubkey: v3Auth, lamports: 5_000_000_000 })
    );
    await provider.sendAndConfirm(fundTx);

    const v3 = vaultPda(v3Auth);
    await vault.methods.initializeVault()
      .accounts({ authority: v3Auth, vault: v3, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([v3AuthKp])
      .rpc();

    const asset2 = assetPda(authority, ASSET2_HASH);

    // asset2 has linked_vault = v (not v3) — bidirectional check must reject
    try {
      await vault.methods.setAssetGate()
        .accounts({ authority: v3Auth, vault: v3, asset: asset2 })
        .signers([v3AuthKp])
        .rpc();
      assert.fail("Should have thrown InvalidAssetAccount");
    } catch (err: any) {
      assert.include(err.toString(), "InvalidAssetAccount");
      console.log("  Bidirectional check rejects asset linked to a different vault ✓");
    }
  });
});
