CONTRACT=secret1c6qft4w76nreh7whn736k58chu8qy9u57rmp89

#secretcli tx compute execute $CONTRACT '{"set_viewing_key":{"key": "123"}}' --from test1 -y --gas 1500000 -b block

#msg=$(base64 -w 0 <<<'{"deposit": {}}')
#secretcli tx compute execute secret12q2c5s5we5zn9pq43l0rlsygtql6646my0sqfm  '{"send":{"recipient": "secret1c6qft4w76nreh7whn736k58chu8qy9u57rmp89", "amount": "2815", "msg": "'"$msg"'"}}' --from test1 -y --gas 1500000 -b block

#secretcli tx compute execute $CONTRACT '{"redeem":{"amount": "2815"}}' --from test1 -y --gas 1500000 -b block


#secretcli q compute query $CONTRACT '{"token_info":{}}'

#secretcli q compute query $CONTRACT '{"reward_token":{}}'

#secretcli q compute query $CONTRACT '{"incentivized_token":{}}'

#secretcli q compute query $CONTRACT '{"total_locked":{}}'

#secretcli q compute query $CONTRACT '{"subscribers":{}}'

secretcli q compute query secret12q2c5s5we5zn9pq43l0rlsygtql6646my0sqfm '{"balance":{"address":"secret1l5sjtktcuh004gsfwht536t6xjru0meas6vhke", "key":"api_key_Ek+JhL74Q3Idt6z3HTUgmwM7aCqvPNlAY1Zi+np69GA="}}'

secretcli q compute query $CONTRACT '{"rewards":{"address":"secret1l5sjtktcuh004gsfwht536t6xjru0meas6vhke", "key":"123", "height": 3694687}}'

secretcli q compute query $CONTRACT '{"balance":{"address":"secret1l5sjtktcuh004gsfwht536t6xjru0meas6vhke", "key":"123"}}'



#secretcli q compute query secret12q2c5s5we5zn9pq43l0rlsygtql6646my0sqfm '{"balance":{"address":"secret18yngramprj5qdaukfwyjl727d8t4cw0j4c00r9", "key":"123"}}'
