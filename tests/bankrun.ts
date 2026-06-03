import { startAnchor, BankrunProvider } from "anchor-bankrun";
import { Clock } from "solana-bankrun";
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { assert } from "chai";

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

    // Accrue interest — should now succeed
    await program.methods
      .accrueInterest()
      .accounts({ vault: vaultPda })
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
        .accounts({ vault: vaultPda })
        .rpc();
      assert.fail("Expected AccrualTooSoon error");
    } catch (err: any) {
      assert.include(JSON.stringify(err), "AccrualTooSoon");
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
      .accounts({ vault: vaultPda })
      .rpc();

    const vault = await program.account.vault.fetch(vaultPda);
    const totalAccrued = BigInt(vault.totalAccruedInterest.toString());
    const expectedPerDay = expectedDailyInterest(DEPOSIT_AMOUNT);

    // Two full days accrued
    assert.equal(totalAccrued, expectedPerDay * 2n);
    console.log(`  Day 2 interest: total accrued = ${totalAccrued} lamports (${expectedPerDay * 2n} expected)`);
  });
});
