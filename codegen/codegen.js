const codegen = require('@cosmwasm/ts-codegen').default;

codegen({
  contracts: [
    {
      name: 'Vault',
      dir: './contracts/provider/vault/schema'
    },
    {
      name: 'ExternalStaking',
      dir: './contracts/provider/external-staking/schema'
    },
    {
      name: 'NativeStaking',
      dir: './contracts/provider/native-staking/schema'
    },
    {
      name: 'NativeStakingProxy',
      dir: './contracts/provider/native-staking-proxy/schema'
    },
    {
      name: 'Converter',
      dir: './contracts/consumer/converter/schema'
    },
    {
      name: 'OsmosisPriceFeed',
      dir: './contracts/consumer/osmosis-price-feed/schema'
    },
    {
      name: 'BandPriceFeed',
      dir: './contracts/consumer/band-price-feed/schema'
    },
    {
      name: 'SimplePriceFeed',
      dir: './contracts/consumer/simple-price-feed/schema'
    },
    {
      name: 'VirtualStaking',
      dir: './contracts/consumer/virtual-staking/schema'
    },
  ],
  outPath: './src/',
  options: {
    bundle: {
      bundleFile: 'index.ts',
      scope: 'contracts'
    },
    messageComposer: {
      enabled: true
    },
    useContractsHooks: {
      enabled: false // if you enable this, add react!
    },
    client: {
      enabled: true
    },
  }
}).then(() => {
  console.log('✨ all done!');
});
