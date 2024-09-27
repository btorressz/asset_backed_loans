describe("Asset Backed Loans Program", () => {
  // Initialize Keypairs for users and accounts
  const user = new web3.Keypair();
  const loanTokenAccount = new web3.Keypair();
  const collateralAccount = new web3.Keypair();

  // SPL Token Program ID
  const SPL_TOKEN_PROGRAM_ID = new web3.PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');

  // Example mint account for testing (you may create a new mint or use an existing one)
  const mint = new web3.Keypair();

  it("Deposits Collateral", async () => {
    const depositAmount = new BN(5000); // Amount of collateral

    // Send transaction to deposit collateral
    const txHash = await pg.program.methods
      .depositCollateral(depositAmount)
      .accounts({
        collateral: collateralAccount.publicKey,
        user: pg.wallet.publicKey,
        userTokenAccount: loanTokenAccount.publicKey,
        systemProgram: web3.SystemProgram.programId,
        tokenProgram: SPL_TOKEN_PROGRAM_ID,  // Correct SPL token program ID
        rent: web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([collateralAccount])
      .rpc();

    console.log(`Transaction hash: ${txHash}`);
    await pg.connection.confirmTransaction(txHash);

    // Fetch collateral account
    const collateralData = await pg.program.account.collateralAccount.fetch(
      collateralAccount.publicKey
    );
    console.log("Collateral on-chain data:", collateralData.amount.toString());

    // Check that the collateral has been correctly deposited
    assert(depositAmount.eq(new BN(collateralData.amount)));
  });

  it("Issues Loan", async () => {
    const loanAmount = new BN(3000); // Loan amount
    const loanDuration = new BN(86400); // 1 day in seconds
    const interestRate = new BN(5); // 5% interest

    // Send transaction to issue a loan
    const txHash = await pg.program.methods
      .issueLoan(loanAmount, loanDuration, interestRate)
      .accounts({
        collateral: collateralAccount.publicKey,
        loanTokenAccount: loanTokenAccount.publicKey,
        loanMint: mint.publicKey,  // Use example mint account
        tokenProgram: SPL_TOKEN_PROGRAM_ID, // Correct SPL token program ID
      })
      .rpc();

    console.log(`Transaction hash: ${txHash}`);
    await pg.connection.confirmTransaction(txHash);

    // Fetch collateral account and check the loan
    const collateralData = await pg.program.account.collateralAccount.fetch(
      collateralAccount.publicKey
    );
    console.log("Loan issued with duration:", collateralData.loanDuration.toString());

    // Check if loan was issued correctly
    assert(loanAmount.lte(new BN(collateralData.amount)));
  });

  it("Repays Loan", async () => {
    const repayAmount = new BN(3200); // Repay full loan + interest

    // Send transaction to repay the loan
    const txHash = await pg.program.methods
      .repayLoan(repayAmount)
      .accounts({
        collateral: collateralAccount.publicKey,
        userTokenAccount: loanTokenAccount.publicKey,
        loanTokenAccount: loanTokenAccount.publicKey,
        tokenProgram: SPL_TOKEN_PROGRAM_ID, // Correct SPL token program ID
      })
      .rpc();

    console.log(`Transaction hash: ${txHash}`);
    await pg.connection.confirmTransaction(txHash);

    // Fetch the collateral account
    const collateralData = await pg.program.account.collateralAccount.fetch(
      collateralAccount.publicKey
    );

    // Check if loan was repaid fully (collateral should be released)
    assert(new BN(collateralData.amount).eq(new BN(0))); // Correct BN comparison
  });

  it("Liquidates Collateral", async () => {
    const liquidator = pg.wallet.publicKey;

    // Liquidate the collateral if loan is not repaid in time
    const txHash = await pg.program.methods
      .liquidateCollateral()
      .accounts({
        collateral: collateralAccount.publicKey,
        liquidator: liquidator,
        tokenProgram: SPL_TOKEN_PROGRAM_ID, // Correct SPL token program ID
      })
      .rpc();

    console.log(`Transaction hash: ${txHash}`);
    await pg.connection.confirmTransaction(txHash);

    // Fetch collateral account
    const collateralData = await pg.program.account.collateralAccount.fetch(
      collateralAccount.publicKey
    );

    // Check that collateral has been liquidated (amount should be 0)
    assert(new BN(collateralData.amount).eq(new BN(0))); // Correct BN comparison
  });
});
