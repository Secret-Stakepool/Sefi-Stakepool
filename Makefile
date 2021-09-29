.PHONY: check
check:
	cargo check

.PHONY: clippy
clippy:
	cargo clippy

PHONY: test
test: unit-test

.PHONY: unit-test
unit-test:
	cargo test

# This is a local build with debug-prints activated. Debug prints only show up
# in the local development chain (see the `start-server` command below)
# and mainnet won't accept contracts built with the feature enabled.
.PHONY: build _build
build: _build compress-wasm
_build:
	RUSTFLAGS='-C link-arg=-s' cargo build --release --target wasm32-unknown-unknown --features="debug-print"

# This is a build suitable for uploading to mainnet.
# Calls to `debug_print` get removed by the compiler.
.PHONY: build-mainnet _build-mainnet
build-mainnet: _build-mainnet compress-wasm
_build-mainnet:
	RUSTFLAGS='-C link-arg=-s' cargo build --release --target wasm32-unknown-unknown

# like build-mainnet, but slower and more deterministic
.PHONY: build-mainnet-reproducible
build-mainnet-reproducible:
	docker run --rm -v "$$(pwd)":/contract \
		--mount type=volume,source="$$(basename "$$(pwd)")_cache",target=/contract/target \
		--mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
		enigmampc/secret-contract-optimizer:1.0.3

.PHONY: compress-wasm
compress-wasm:
	cp ./target/wasm32-unknown-unknown/release/*.wasm ./contract.wasm
	@## The following line is not necessary, may work only on linux (extra size optimization)
	@# wasm-opt -Os ./contract.wasm -o ./contract.wasm
	cat ./contract.wasm | gzip -9 > ./contract.wasm.gz

.PHONY: schema
schema:
	cargo run --example schema

# Run local development chain with four funded accounts (named a, b, c, and d)
.PHONY: start-server
start-server: # CTRL+C to stop
	docker run -it --rm \
		-p 26657:26657 -p 26656:26656 -p 1317:1317 \
		-v $$(pwd):/root/code \
		--name secretdev enigmampc/secret-network-sw-dev:v1.0.4-3

# This relies on running `start-server` in another console
# You can run other commands on the secretcli inside the dev image
# by using `docker exec secretdev secretcli`.
.PHONY: store-contract-local
store-contract-local:
	docker exec secretdev secretcli tx compute store -y --from a --gas 10000000 /root/code/contract.wasm.gz

.PHONY: list-code
list-code:
	docker exec secretdev secretcli query compute list-code

#make instanciate-local CODE=1
.PHONY: instanciate-local
instanciate-local:
	docker exec secretdev secretcli tx compute instantiate $(CODE) \
	"{ \
		\"prng_seed\": \"ZW5pZ21hLXJvY2tzCg==\", \
		\"triggerer\": \"secret1v5y7as75cqd0trtq62hgzj7u4ck9slhnrf3k4c\", \
		\"token\": { \"address\": \"secret1ypfxpp4ev2sd9vj9ygmsmfxul25xt9cfadrxxy\", \"contract_hash\": \"0xb66c6aca95004916baa13f8913ff1222c3e1775aaaf60f011cfaba7296d59d2c\"}, \
		\"staking_contract\": { \"address\": \"secret1c6qft4w76nreh7whn736k58chu8qy9u57rmp89\", \"contract_hash\": \"0x8fcc4c975a67178b8b15b903f99604c2a38be118bcb35751ffde9183a2c6a193\"}, \
		\"viewing_key\": \"123\", \
		\"ticket_price\": \"10\", \
		\"base_reward_pot_allocations\": {\"burn\": 15, \"triggerer\": 1, \"sequence_1\": 2, \"sequence_2\": 4, \"sequence_3\": 6, \"sequence_4\": 12, \"sequence_5\": 20, \"sequence_6\": 40  }, \
		\"minimum_next_round_allocation\": 10, \
		\"per_ticket_bulk_discount\": \"25000\", \
		\"min_round_trigger_in_blocks\": 10 \
	}" \
	--from a --gas 15000000 --label $(CODE) -b block -y

.PHONY: clean
clean:
	cargo clean
	-rm -f ./contract.wasm ./contract.wasm.gz

#make hashes TX=99548FEB8D07C75E475814CA5A6FAD707893D80198E28A01FA1898C8D0FFCA4E
.PHONY: hashes
hashes:
	secretcli query compute tx $(TX)
	
.PHONY: deploy
deploy:
	bash deploy/testnet.sh