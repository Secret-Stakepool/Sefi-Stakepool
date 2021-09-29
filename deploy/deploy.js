const {
  EnigmaUtils, Secp256k1Pen, SigningCosmWasmClient, pubkeyToAddress, encodeSecp256k1Pubkey,
} = require('secretjs');

const fs = require('fs');

// Load environment variables
require('dotenv').config();

const customFees = {
  upload: {
    amount: [{ amount: '2000000', denom: 'uscrt' }],
    gas: '2200000',
  },
  init: {
    amount: [{ amount: '500000', denom: 'uscrt' }],
    gas: '700000',
  },
  exec: {
    amount: [{ amount: '500000', denom: 'uscrt' }],
    gas: '500000',
  },
  send: {
    amount: [{ amount: '80000', denom: 'uscrt' }],
    gas: '80000',
  },
};

const main = async () => {
  const httpUrl = process.env.SECRET_REST_URL;

  // Use key created in tutorial #2
  const mnemonic = process.env.MNEMONIC;

  // A pen is the most basic tool you can think of for signing.
  // This wraps a single keypair and allows for signing.
  const signingPen = await Secp256k1Pen.fromMnemonic(mnemonic)
    .catch((err) => { throw new Error('Could not get signing pen: ${err}'); });

  // Get the public key
  const pubkey = encodeSecp256k1Pubkey(signingPen.pubkey);

  // get the wallet address
  const accAddress = pubkeyToAddress(pubkey, 'secret');

  // 1. Initialize client
  const txEncryptionSeed = EnigmaUtils.GenerateNewSeed();

  const client = new SigningCosmWasmClient(
    httpUrl,
    accAddress,
    (signBytes) => signingPen.sign(signBytes),
    txEncryptionSeed, customFees,
  );
  console.log(`Wallet address=${accAddress}`);
  // 2. Upload the contract wasm

  const wasm = fs.readFileSync('/Users/haseebsaeed/codes/sefi-stakepool-v2/contract.wasm');
  console.log('Uploading contract');
  const uploadReceipt = await client.upload(wasm, {})
    .catch((err) => { throw new Error(`Could not upload contract: ${err}`); });

  // 3. Create an instance of the Counter contract
  // Get the code ID from the receipt
  const { codeId } = uploadReceipt;

  // Create an instance of the Counter contract, providing a starting count //change
  const initMsg = { "token":{"address":"secret12q2c5s5we5zn9pq43l0rlsygtql6646my0sqfm","contract_hash":"c7fe67b243dfedc625a28ada303434d6f5a46a3086e7d2b5063a814e9f9a379d"},
    "staking_contract":{"address":"secret1c6qft4w76nreh7whn736k58chu8qy9u57rmp89","contract_hash":"8fcc4c975a67178b8b15b903f99604c2a38be118bcb35751ffde9183a2c6a193" },
    "viewing_key": "123",
    "token_info":{"name":"Sefi","symbol":"sSefi"},
    "prng_seed": "ZW5pZ21hLXJvY2tzCg==",
    "admin":"secret14v6h248vatcsur9hwqjekvj7t6jd8anf8ykw4n",
    "triggerer":"secret1uzzzzr02xk9cuxn6ejp2axsyf4cjzznklzjmq7"  };

  const contract = await client.instantiate(codeId, initMsg, 'Sefi_Stakepool_v13')
    .catch((err) => { throw new Error(`Could not instantiate contract: ${err}`); });
  const { contractAddress } = contract;
  console.log('contract: ', contract);

  // // 4. Query the counter

  // // 5. Increment the counter

  // Query again to confirm it worked
  console.log('Querying contract for updated count');
  response = await client.queryContractSmart(contractAddress, { lottery_info: {} })
    .catch((err) => { throw new Error(`Could not query contract: ${err}`); });

  console.log(`New Count=${response.lottery_info.start_height}`);
};

main().catch((err) => {
  console.error(err);
});
