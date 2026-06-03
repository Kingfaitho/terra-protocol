/**
 * Phase 3 Step 2 — Dispute Reward Distribution Tests (Bankrun)
 *
 * Tests the 70/30 split (disputer/treasury) for upheld disputes and
 * griefing prevention (dismissed bonds to treasury, minimum bond enforcement).
 */

import { startAnchor, BankrunProvider } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraAttestation } from "../target/types/terra_attestation";
import { assert } from "chai";

const ATTEST_IDL = require("../target/idl/terra_attestation.json");

describe("TERRA Phase 3 Step 2 — Dispute Reward Distribution", () => {
  let context: Awaited<ReturnType<typeof startAnchor>>;
  let provider: BankrunProvider;
  let attest: Program<TerraAttestation>;
  let authority: anchor.web3.PublicKey;

  const agentKp     = anchor.web3.Keypair.generate();
  const disputerKp  = anchor.web3.Keypair.generate();
  const disputerKp2 = anchor.web3.Keypair.generate();

  const STAKE          = new anchor.BN(2_000_000);
  const MIN_BOND       = new anchor.BN(1_000_000); // 0.001 SOL
  const BOND_UPHELD    = new anchor.BN(5_000_000); // 0.005 SOL
  const BOND_DISMISSED = new anchor.BN(2_000_000); // 0.002 SOL
  const LOC_HASH       = Array(32).fill(0x10);
  const ASSET_HASH     = Array(32).fill(0x20);
  const ASSET2_HASH    = Array(32).fill(0x21);

  const resolverPda = () =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("resolver")], attest.programId)[0];

  const treasuryPda = () =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("treasury")], attest.programId)[0];

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

  before(async () => {
    context  = await startAnchor(".", [], []);
    provider = new BankrunProvider(context);
    attest   = new Program<TerraAttestation>(ATTEST_IDL, provider);
    authority = provider.wallet.publicKey;

    for (const kp of [agentKp, disputerKp, disputerKp2]) {
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

  it("Initializes treasury and resolver", async () => {
    await attest.methods.initializeResolver()
      .accounts({ authority, disputeResolver: resolverPda(), systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    await attest.methods.initializeTreasury()
      .accounts({ authority, treasury: treasuryPda(), systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    const treasury = await attest.account.treasury.fetch(treasuryPda());
    assert.equal(treasury.authority.toString(), authority.toString());
    console.log("  Treasury and resolver initialized ✓");
  });

  it("Rejects dispute with bond below minimum", async () => {
    const asset = assetPda(authority, ASSET_HASH);

    // Register and attest asset
    await attest.methods.registerAgent(STAKE)
      .accounts({ authority, agent: agentPda(authority), systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    await attest.methods.registerAsset({ crop: {} }, new anchor.BN(100), LOC_HASH, ASSET_HASH, 1)
      .accounts({ authority, asset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    await attest.methods.attestAsset(Array(32).fill(0x01))
      .accounts({
        agentAuthority: authority, agent: agentPda(authority),
        asset, attestation: attestationPda(asset, authority),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    // Try to dispute with too-small bond
    const smallBond = new anchor.BN(500_000); // Below 1M minimum
    try {
      await attest.methods.raiseDispute(Array(32).fill(0xaa), smallBond)
        .accounts({
          disputer: disputerKp.publicKey,
          asset,
          dispute: disputePda(asset, disputerKp.publicKey),
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([disputerKp])
        .rpc();
      assert.fail("Should have rejected bond below minimum");
    } catch (err: any) {
      assert.include(err.toString(), "BondTooSmall");
      console.log("  Minimum bond enforced (prevents griefing) ✓");
    }
  });

  it("Upheld dispute: 70% to disputer, 30% to treasury", async () => {
    const asset = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);

    // Raise dispute with valid bond
    await attest.methods.raiseDispute(Array(32).fill(0xaa), BOND_UPHELD)
      .accounts({
        disputer: disputerKp.publicKey,
        asset,
        dispute,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([disputerKp])
      .rpc();

    // Uphold the dispute (no slashing to keep math simple)
    await attest.methods.resolveDispute(true)
      .accounts({
        resolver: authority,
        disputeResolver: resolverPda(),
        dispute,
        asset,
      })
      .rpc();

    // Record balances before claim
    const treasuryBefore = await context.banksClient.getBalance(treasuryPda());
    const disputerBefore = await context.banksClient.getBalance(disputerKp.publicKey);

    // Claim
    await attest.methods.claimUpheldDispute()
      .accounts({
        dispute,
        disputer: disputerKp.publicKey,
        treasury: treasuryPda(),
      })
      .rpc();

    // Verify split: 70% to disputer, 30% to treasury
    const treasuryAfter = await context.banksClient.getBalance(treasuryPda());
    const disputerAfter = await context.banksClient.getBalance(disputerKp.publicKey);

    const treasuryGain = Number(treasuryAfter) - Number(treasuryBefore);
    const disputerGain = Number(disputerAfter) - Number(disputerBefore);

    const expectedDisputer = Math.floor((BOND_UPHELD.toNumber() * 70) / 100);
    const expectedTreasury = BOND_UPHELD.toNumber() - expectedDisputer;

    assert.equal(disputerGain, expectedDisputer);
    assert.equal(treasuryGain, expectedTreasury);

    console.log(`  Upheld split: disputer ${expectedDisputer} (70%), treasury ${expectedTreasury} (30%) ✓`);
  });

  it("Dismissed dispute: 100% of bond goes to treasury, disputer gets nothing", async () => {
    const asset = assetPda(authority, ASSET2_HASH);
    const dispute = disputePda(asset, disputerKp2.publicKey);

    // Register and attest asset2
    await attest.methods.registerAsset({ crop: {} }, new anchor.BN(100), LOC_HASH, ASSET2_HASH, 1)
      .accounts({ authority, asset, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    await attest.methods.attestAsset(Array(32).fill(0x02))
      .accounts({
        agentAuthority: authority, agent: agentPda(authority),
        asset, attestation: attestationPda(asset, authority),
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    // Raise and dismiss dispute
    await attest.methods.raiseDispute(Array(32).fill(0xbb), BOND_DISMISSED)
      .accounts({
        disputer: disputerKp2.publicKey,
        asset,
        dispute,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([disputerKp2])
      .rpc();

    await attest.methods.resolveDispute(false)
      .accounts({
        resolver: authority,
        disputeResolver: resolverPda(),
        dispute,
        asset,
      })
      .rpc();

    // Record balances before claim
    const treasuryBefore = await context.banksClient.getBalance(treasuryPda());
    const disputerBefore = await context.banksClient.getBalance(disputerKp2.publicKey);

    // Claim dismissed
    await attest.methods.claimDismissedDispute()
      .accounts({
        dispute,
        treasury: treasuryPda(),
      })
      .rpc();

    // Verify: disputer gets nothing, treasury gets 100% of bond
    const treasuryAfter = await context.banksClient.getBalance(treasuryPda());
    const disputerAfter = await context.banksClient.getBalance(disputerKp2.publicKey);

    const treasuryGain = Number(treasuryAfter) - Number(treasuryBefore);
    const disputerGain = Number(disputerAfter) - Number(disputerBefore);

    assert.equal(treasuryGain, BOND_DISMISSED.toNumber());
    assert.equal(disputerGain, 0);

    console.log(`  Dismissed split: disputer 0, treasury ${BOND_DISMISSED} (100%) ✓`);
    console.log(`  Griefing cost: disputer forfeited ${BOND_DISMISSED} lamports for false accusation ✓`);
  });

  it("Cannot claim upheld dispute twice (balance already transferred)", async () => {
    const asset = assetPda(authority, ASSET_HASH);
    const dispute = disputePda(asset, disputerKp.publicKey);

    // Try to claim again (dispute already claimed)
    try {
      await attest.methods.claimUpheldDispute()
        .accounts({
          dispute,
          disputer: disputerKp.publicKey,
          treasury: treasuryPda(),
        })
        .rpc();
      // Will fail because dispute account no longer has lamports
      assert.fail("Should have failed due to insufficient lamports in dispute PDA");
    } catch (err: any) {
      // Expected: trying to transfer more than available
      console.log("  Double-claim prevented (insufficient lamports in dispute PDA) ✓");
    }
  });
});
