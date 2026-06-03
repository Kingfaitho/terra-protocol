/**
 * Multi-depositor interest accounting — adversarial test
 *
 * Proves the current pro-rata formula has no time-weighting:
 * Alice deposits early, earns interest for a day alone.
 * Bob deposits later (9× larger), then immediately dilutes Alice's earnings.
 *
 * This test FAILS today. It defines the invariant the fix must satisfy:
 *   - Bob cannot withdraw interest earned before he joined
 *   - Alice receives the interest she earned while the only depositor
 *   - Σ payouts ≤ funded pool (no money creation)
 */

import { startAnchor, BankrunProvider } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { assert } from "chai";

const VAULT_IDL = require("../target/idl/terra_vault.json");

describe("TERRA — Multi-depositor Interest Fairness", () => {
  let context: Awaited<ReturnType<typeof startAnchor>>;
  let provider: BankrunProvider;
  let vault: Program<TerraVault>;
  let authority: anchor.web3.PublicKey;

  const aliceKp = anchor.web3.Keypair.generate();
  const bobKp   = anchor.web3.Keypair.generate();

  const ALICE_DEPOSIT = new anchor.BN(1_000_000);  // 0.001 SOL
  const BOB_DEPOSIT   = new anchor.BN(9_000_000);  // 0.009 SOL  (9× Alice)
  const FUND_AMOUNT   = new anchor.BN(30);          // 30 lamports yield pool

  const vaultPda = (auth: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), auth.toBuffer()], vault.programId)[0];

  const depositPda = (v: anchor.web3.PublicKey, dep: anchor.web3.PublicKey) =>
    anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), v.toBuffer(), dep.toBuffer()], vault.programId)[0];

  before(async () => {
    context  = await startAnchor(".", [], []);
    provider = new BankrunProvider(context);
    vault    = new Program<TerraVault>(VAULT_IDL, provider);
    authority = provider.wallet.publicKey;

    for (const kp of [aliceKp, bobKp]) {
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

  it("Interest is time-weighted: Alice earns day-1 interest, Bob only earns from when he joined", async () => {
    const v         = vaultPda(authority);
    const aliceDep  = depositPda(v, aliceKp.publicKey);
    const bobDep    = depositPda(v, bobKp.publicKey);

    // ── Setup vault ───────────────────────────────────────────────────────
    await vault.methods.initializeVault()
      .accounts({ authority, vault: v, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // ── Alice deposits 1M lamports ────────────────────────────────────────
    await vault.methods.deposit(ALICE_DEPOSIT)
      .accounts({ vault: v, vaultDeposit: aliceDep, depositor: aliceKp.publicKey, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([aliceKp])
      .rpc();

    // ── Day 1: warp +25h, accrue. Only Alice in vault (1M deposits). ──────
    // Expected daily interest on 1M at 8% APY = (1M * 800) / 36500 / 10000 = 2 lamports
    let clock = await context.banksClient.getClock();
    context.setClock(new Clock(clock.slot, clock.epochStartTimestamp, clock.epoch,
      clock.leaderScheduleEpoch, clock.unixTimestamp + BigInt(25 * 3600)));

    await vault.methods.accrueInterest()
      .accounts({ vault: v, asset: v }) // no gate — vault key as sentinel
      .rpc();

    const afterDay1 = await vault.account.vault.fetch(v);
    const day1Interest = afterDay1.totalAccruedInterest.toNumber();
    // 2 lamports — Alice earned this alone
    assert.equal(day1Interest, 2, `Day 1: expected 2 lamports accrued, got ${day1Interest}`);
    console.log(`  Day 1 interest (Alice alone): ${day1Interest} lamports`);

    // ── Bob deposits 9M — after Alice's day-1 interest already accrued ────
    await vault.methods.deposit(BOB_DEPOSIT)
      .accounts({ vault: v, vaultDeposit: bobDep, depositor: bobKp.publicKey, systemProgram: anchor.web3.SystemProgram.programId })
      .signers([bobKp])
      .rpc();

    // ── Day 2: warp +25h, accrue. Both depositors (10M total). ───────────
    // Expected daily interest on 10M = (10M * 800) / 36500 / 10000 = 21 lamports
    clock = await context.banksClient.getClock();
    context.setClock(new Clock(clock.slot, clock.epochStartTimestamp, clock.epoch,
      clock.leaderScheduleEpoch, clock.unixTimestamp + BigInt(25 * 3600)));

    await vault.methods.accrueInterest()
      .accounts({ vault: v, asset: v })
      .rpc();

    const afterDay2 = await vault.account.vault.fetch(v);
    const totalAccrued = afterDay2.totalAccruedInterest.toNumber();
    // 2 (day 1) + 21 (day 2) = 23 lamports total
    assert.equal(totalAccrued, 23, `Expected 23 total accrued, got ${totalAccrued}`);
    console.log(`  Day 2 accrued: 21 lamports. Total accrued: ${totalAccrued} lamports`);

    // ── Fund the vault with 30 lamports yield pool ────────────────────────
    await vault.methods.fundVaultInterest(FUND_AMOUNT)
      .accounts({ authority, vault: v, systemProgram: anchor.web3.SystemProgram.programId })
      .rpc();

    // ── Bob withdraws 9M ──────────────────────────────────────────────────
    const bobBalBefore = await context.banksClient.getBalance(bobKp.publicKey);
    await vault.methods.withdraw(BOB_DEPOSIT)
      .accounts({ vault: v, vaultDeposit: bobDep, depositor: bobKp.publicKey })
      .signers([bobKp])
      .rpc();
    const bobBalAfter = await context.banksClient.getBalance(bobKp.publicKey);
    const bobReceived  = Number(bobBalAfter) - Number(bobBalBefore);
    const bobInterest  = bobReceived - BOB_DEPOSIT.toNumber();

    // ── Alice withdraws 1M ────────────────────────────────────────────────
    const aliceBalBefore = await context.banksClient.getBalance(aliceKp.publicKey);
    await vault.methods.withdraw(ALICE_DEPOSIT)
      .accounts({ vault: v, vaultDeposit: aliceDep, depositor: aliceKp.publicKey })
      .signers([aliceKp])
      .rpc();
    const aliceBalAfter = await context.banksClient.getBalance(aliceKp.publicKey);
    const aliceReceived = Number(aliceBalAfter) - Number(aliceBalBefore);
    const aliceInterest = aliceReceived - ALICE_DEPOSIT.toNumber();

    console.log(`  Bob interest received:   ${bobInterest} lamports`);
    console.log(`  Alice interest received: ${aliceInterest} lamports`);
    console.log(`  Total interest paid out: ${bobInterest + aliceInterest} lamports`);

    // ── Invariant checks ─────────────────────────────────────────────────
    //
    // Correct time-weighted allocation:
    //   Day 1 (Alice alone, 1M/1M = 100%): Alice earns 2 lamports
    //   Day 2 (Alice 1M/10M = 10%, Bob 9M/10M = 90%):
    //     Alice earns 21 * 10% = 2.1 → 2 lamports
    //     Bob earns  21 * 90% = 18.9 → 18 lamports
    //
    // Expected totals: Alice = 4, Bob = 18, total paid = 22 (1 lost to rounding)
    //
    // BROKEN allocation (current code):
    //   Bob withdrawal: (9M/10M) * 23 = 20.7 → 20 lamports  ← steals Alice's day-1
    //   Alice withdrawal: (1M/10M) * (23-20) = 0.3 → 0 lamports ← Alice gets nothing

    // Invariant 1: total payout must not exceed funded pool
    assert(bobInterest + aliceInterest <= FUND_AMOUNT.toNumber(),
      `INVARIANT VIOLATED: paid out ${bobInterest + aliceInterest} > funded ${FUND_AMOUNT}`);

    // Invariant 2: Bob must NOT receive more interest than he earned (day 2 only, 90% share)
    // Bob's maximum fair share = 90% of day-2 interest = 18 lamports
    const bobMaxFair = 18;
    assert(bobInterest <= bobMaxFair,
      `FAIRNESS VIOLATED: Bob received ${bobInterest} lamports but his fair share is ≤${bobMaxFair} lamports (he didn't earn day-1 interest)`);

    // Invariant 3: Alice must receive at least day-1 interest she earned alone
    const aliceMinFair = 2; // She earned 2 lamports on day 1 as the sole depositor
    assert(aliceInterest >= aliceMinFair,
      `FAIRNESS VIOLATED: Alice received ${aliceInterest} lamports but she earned ≥${aliceMinFair} lamports before Bob joined`);

    console.log(`  ✓ Fairness invariants satisfied`);
    console.log(`  Bob: ${bobInterest} ≤ ${bobMaxFair} (day-2 max) ✓`);
    console.log(`  Alice: ${aliceInterest} ≥ ${aliceMinFair} (day-1 min) ✓`);
  });
});
