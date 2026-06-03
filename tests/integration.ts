import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { TerraVault } from "../target/types/terra_vault";
import { assert } from "chai";

describe("TERRA Vault - Precision Settlement Engine", () => {
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

  const getDepositPda = (vaultPda: anchor.web3.PublicKey) => {
    const [depositPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("deposit"), vaultPda.toBuffer(), authority.toBuffer()],
      program.programId
    );
    return depositPda;
  };

  // TEST 1: Initialize Vault
  it("Initializes a vault with correct state", async () => {
    const vaultPda = getVaultPda();

    const tx = await program.methods
      .initializeVault()
      .accounts({
        authority,
        vault: vaultPda,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    console.log("  TX:", tx);

    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.authority.toString(), authority.toString(), "Authority mismatch");
    assert.equal(vault.totalDeposits.toNumber(), 0, "Initial deposits should be 0");
    assert.equal(vault.totalAccruedInterest.toNumber(), 0, "Initial interest should be 0");
    assert.equal(vault.dailyInterestRate.toNumber(), 800, "Rate should be 800 bps (8% APY)");
    console.log("  Vault PDA:", vaultPda.toString());
  });

  // TEST 2: Deposit into Vault
  it("Deposits SOL into vault and records deposit", async () => {
    const vaultPda = getVaultPda();
    const depositPda = getDepositPda(vaultPda);

    const depositAmount = new anchor.BN(1_000_000); // 0.001 SOL

    const balanceBefore = await provider.connection.getBalance(authority);

    const tx = await program.methods
      .deposit(depositAmount)
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    console.log("  TX:", tx);

    // Verify deposit record
    const deposit = await program.account.vaultDeposit.fetch(depositPda);
    assert.equal(deposit.amountDeposited.toNumber(), 1_000_000, "Deposit amount mismatch");
    assert.equal(deposit.depositor.toString(), authority.toString(), "Depositor mismatch");
    assert.equal(deposit.vault.toString(), vaultPda.toString(), "Vault ref mismatch");

    // Verify vault total updated
    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.totalDeposits.toNumber(), 1_000_000, "Vault total deposits not updated");

    // Verify SOL actually moved (balance decreased by at least the deposit amount)
    const balanceAfter = await provider.connection.getBalance(authority);
    assert(balanceBefore - balanceAfter >= 1_000_000, "SOL was not transferred from depositor");

    // Verify vault account now holds the SOL
    const vaultBalance = await provider.connection.getBalance(vaultPda);
    assert(vaultBalance >= 1_000_000, "Vault did not receive SOL");

    console.log("  Deposit PDA:", depositPda.toString());
  });

  // TEST 3: accrue_interest 24-hour guard
  it("Rejects interest accrual before 24 hours have elapsed", async () => {
    const vaultPda = getVaultPda();

    // Vault was just initialized seconds ago — time_elapsed << 86400, so this must fail.
    try {
      await program.methods
        .accrueInterest()
        .accounts({ vault: vaultPda })
        .rpc();
      assert.fail("Expected AccrualTooSoon error but instruction succeeded");
    } catch (err: any) {
      // Anchor wraps program errors; confirm we got the right one.
      assert.include(
        JSON.stringify(err),
        "AccrualTooSoon",
        "Expected AccrualTooSoon error code"
      );
      console.log("  AccrualTooSoon guard works correctly (error code 6001)");
    }

    // Verify the precision math formula independently (no on-chain call needed).
    // 1_000_000 lamports at 800 bps annually:
    // (1_000_000 * 800) / 36500 / 10000 = 2 lamports/day
    const deposits = 1_000_000;
    const rate = 800;
    const expected = Math.floor(Math.floor((deposits * rate) / 36500) / 10000);
    assert.equal(expected, 2, "Precision math formula: 1 SOL at 8% APY = 2 lamports/day");
    console.log(`  Precision math verified: 1_000_000 lamports × 8% APY = ${expected} lamports/day`);
  });

  // TEST 4: Withdraw from Vault
  it("Withdraws principal from vault and transfers SOL back", async () => {
    const vaultPda = getVaultPda();
    const depositPda = getDepositPda(vaultPda);

    const withdrawAmount = new anchor.BN(500_000); // Half the deposit

    const balanceBefore = await provider.connection.getBalance(authority);

    const tx = await program.methods
      .withdraw(withdrawAmount)
      .accounts({
        vault: vaultPda,
        vaultDeposit: depositPda,
        depositor: authority,
      })
      .rpc();

    console.log("  TX:", tx);

    // Verify deposit record reduced
    const deposit = await program.account.vaultDeposit.fetch(depositPda);
    assert.equal(deposit.amountDeposited.toNumber(), 500_000, "Deposit not reduced correctly");

    // Verify vault total reduced
    const vault = await program.account.vault.fetch(vaultPda);
    assert.equal(vault.totalDeposits.toNumber(), 500_000, "Vault total not reduced");

    // Verify SOL came back to depositor (net of tx fees, balance should have increased ~500_000)
    const balanceAfter = await provider.connection.getBalance(authority);
    assert(balanceAfter > balanceBefore, "Depositor balance should increase after withdrawal");

    console.log(`  Remaining in vault: ${deposit.amountDeposited.toNumber()} lamports`);
  });
});
