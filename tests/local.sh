CONTRACT=secret10pyejy66429refv3g35g2t7am0was7ya6hvrzf
WALLET=secret1vg68hc7plfn5hwvgjqgd27jynckfjnsp3tyz5n

# Create VK
#docker exec secretdev secretcli tx compute execute $CONTRACT '{"create_viewing_key":{"entropy": "123"}}' --from a -y --gas 1500000 -b block

# Set VK
#docker exec secretdev secretcli tx compute execute $CONTRACT '{"set_viewing_key":{"key": "123"}}' --from a -y --gas 1500000 -b block

# Change Triggerer
#docker exec secretdev secretcli tx compute execute $CONTRACT '{"change_triggerer":{"triggerer": "secret1ypfxpp4ev2sd9vj9ygmsmfxul25xt9cfadrxxy"}}' --from a -y --gas 1500000 -b block

# Change Admin
#docker exec secretdev secretcli tx compute execute $CONTRACT '{"change_admin":{"admin": "secret1ypfxpp4ev2sd9vj9ygmsmfxul25xt9cfadrxxy"}}' --from a -y --gas 1500000 -b block

# Buy Tickets
msg=$(base64 -w 0 <<<'{"buy_tickets": {"tickets": ["000000","100000","200000","300000","400000","500000","600000","700000","800000","900000"], "entropy": "'"$RANDOM"'"}}')
docker exec secretdev secretcli tx compute execute $CONTRACT '{"receive":{"sender": "secret1ypfxpp4ev2sd9vj9ygmsmfxul25xt9cfadrxxy", "from": "secret1ypfxpp4ev2sd9vj9ygmsmfxul25xt9cfadrxxy", "amount": "100", "msg": "'"$msg"'"}}' --from a -y --gas 1500000 -b block

# Trigger
docker exec secretdev secretcli tx compute execute $CONTRACT '{"trigger_end_round":{"entropy": "'"$RANDOM"'"}}' --from a -y --gas 1500000 -b block

# Query Configs
#docker exec secretdev secretcli q compute query $CONTRACT '{"get_configs":{}}' | base64 --decode --ignore-garbage

# Query Rounds
docker exec secretdev secretcli q compute query $CONTRACT '{"get_rounds":{"round_numbers": [9]}}' | base64 --decode --ignore-garbage

# Query User Tickets
#docker exec secretdev secretcli q compute query $CONTRACT '{"get_user_tickets":{"address": "'"${WALLET}"'", "key": "123", "round_numbers": [0]}}' | base64 --decode --ignore-garbage
