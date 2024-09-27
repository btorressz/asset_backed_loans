(async () => {
  // Log the wallet's public address
  console.log("My address:", pg.wallet.publicKey.toString());

  // Get the wallet's balance in SOL
  const balance = await pg.connection.getBalance(pg.wallet.publicKey);
  console.log(`My balance: ${balance / web3.LAMPORTS_PER_SOL} SOL`);

  // Example of how to fetch an account's data (replace with actual program account)
  const accountPublicKey = new web3.PublicKey("YourProgramAccountPubkeyHere");

  try {
    const accountData = await pg.connection.getAccountInfo(accountPublicKey);

    if (accountData) {
      console.log("Account data:", accountData.data.toString());
    } else {
      console.log("No data found for the provided account.");
    }
  } catch (err) {
    console.error("Error fetching account info:", err);
  }

  // Example of sending a transaction (replace with actual program method)
  const transaction = await pg.program.methods
    .yourMethodNameHere(new web3.BN(1234)) // Replace with appropriate method and params
    .accounts({
      signer: pg.wallet.publicKey,
      // Add required accounts for the transaction here
    })
    .rpc();

  console.log(`Transaction sent: ${transaction}`);
})();
