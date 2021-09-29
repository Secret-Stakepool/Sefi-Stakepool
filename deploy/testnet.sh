
#!/bin/bash
echo Build new contracts to deploy? [yn]
read toBuild

function wait_for_tx() {
  until (secretcli q tx "$1"); do
      sleep 5
  done
}

if [ "$toBuild" != "${toBuild#[Yy]}" ] ;then
    RUST_BACKTRACE=1 cargo unit-test
    rm -f ./contract.wasm ./contract.wasm.gz
    cargo wasm
    cargo schema
    docker run --rm -v $PWD:/contract \
        --mount type=volume,source=factory_cache,target=/code/target \
        --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
        enigmampc/secret-contract-optimizer --platform linux/amd64
fi

docker run --rm -v "$(pwd)":/contract \
  --mount type=volume,source="$(basename "$(pwd)")_cache",target=/code/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  enigmampc/secret-contract-optimizer --platform linux/amd64

secretcli q account $(secretcli keys show -a test1)

deployed=$(secretcli tx compute store "contract.wasm.gz" --from test1 --gas 2500000 -b block -y)
code_id=$(secretcli query compute list-code | jq '.[-1]."id"')
code_hash=$(secretcli query compute list-code | jq '.[-1]."data_hash"')
echo "Stored contract: '$code_id', '$code_hash'"


label=$(date +"%T")
STORE_TX_HASH=$(
  secretcli tx compute instantiate $code_id " \
  { \
    \"prng_seed\": \"ZW5pZ21hLXJvY2tzCg==\", \
    \"triggerer\": \"secret1l5sjtktcuh004gsfwht536t6xjru0meas6vhke\", \
	  \"token\": { \"address\": \"secret12q2c5s5we5zn9pq43l0rlsygtql6646my0sqfm\", \"contract_hash\": \"c7fe67b243dfedc625a28ada303434d6f5a46a3086e7d2b5063a814e9f9a379d\"}, \
	  \"staking_contract\": { \"address\": \"secret1c6qft4w76nreh7whn736k58chu8qy9u57rmp89\", \"contract_hash\": \"8fcc4c975a67178b8b15b903f99604c2a38be118bcb35751ffde9183a2c6a193\"}, \
	  \"viewing_key\": \"123\", \
	  \"ticket_price\": \"100\", \
	  \"base_reward_pot_allocations\": {\"burn\": 15, \"triggerer\": 1, \"sequence_1\": 2, \"sequence_2\": 4, \"sequence_3\": 6, \"sequence_4\": 12, \"sequence_5\": 20, \"sequence_6\": 40  }, \
    \"minimum_next_round_allocation\": 10, \
    \"per_ticket_bulk_discount\": \"25000\", \
    \"min_round_trigger_in_blocks\": 5 \
  } \
  " --from test1 --gas 15000000 --label SecretLottery_$label -b block -y |
  jq -r .txhash
)

wait_for_tx "$STORE_TX_HASH" "Waiting for instantiate to finish on-chain..."

contract_address=$(secretcli query compute list-contract-by-code $code_id | jq '.[-1].address')
echo "contract_address: '$contract_address'"
contract_address_without_quotes=$(echo $contract_address | tr -d '"')


# Create VK
# secretcli tx compute execute $CONTRACT '{"create_viewing_key":{"entropy": "123"}}' --from test1 -y --gas 1500000 -b block

# Set VK
#secretcli tx compute execute $contract_address_without_quotes '{"set_viewing_key":{"key": "123"}}' --from test1 -y --gas 1500000 -b block

# Buy Tickets
#msg=$(base64 -w 0 <<<'{"buy_tickets": {"tickets": ["000000"], "entropy": "'"$RANDOM"'"}}')
#secretcli tx compute execute secret12q2c5s5we5zn9pq43l0rlsygtql6646my0sqfm  '{"send":{"recipient": '$contract_address', "amount": "100", "msg": "'"$msg"'"}}' --from test1 -y --gas 1500000 -b block

# Trigger
#secretcli tx compute execute $contract_address_without_quotes '{"trigger_end_round":{"entropy": "'"$RANDOM"'"}}' --from test1 -y --gas 1500000 -b block

# Change Allocations
#secretcli tx compute execute $contract_address_without_quotes '{"change_base_reward_pool_allocations": {"minimum_next_round_allocation": 10, "base_reward_pot_allocations": {"burn": 100, "triggerer": 0, "sequence_1": 0, "sequence_2": 0, "sequence_3": 0, "sequence_4": 0, "sequence_5": 0, "sequence_6": 0  }}}' --from test1 -y --gas 1500000 -b block

# Change Ticket Price
#secretcli tx compute execute $contract_address_without_quotes '{"change_base_ticket_price": {"base_ticket_price": 1 }}' --from test1 -y --gas 1500000 -b block

# Claim Reward
#secretcli tx compute execute $contract_address_without_quotes '{"claim_rewards":{"round": 1, "tickets_index": [5]}}' --from test1 -y --gas 1500000 -b block

# Query Configs
# secretcli q compute query $contract_address_without_quotes '{"get_configs":{}}' | base64 --decode --ignore-garbage

# Query Rounds
#secretcli q compute query $contract_address_without_quotes '{"get_rounds":{"round_numbers": [0,1]}}' | base64 --decode --ignore-garbage

# Query User Tickets
#secretcli q compute query $contract_address_without_quotes '{"get_user_tickets":{"address": "secret1l5sjtktcuh004gsfwht536t6xjru0meas6vhke", "key": "123", "round_numbers": [0]}}' | base64 --decode --ignore-garbage

# Query Paginatted User Tickets
#secretcli q compute query $contract_address_without_quotes '{"get_paginated_user_tickets":{"address": "secret1l5sjtktcuh004gsfwht536t6xjru0meas6vhke", "key": "123", "page": 0, "page_size": 5}}' | base64 --decode --ignore-garbage
