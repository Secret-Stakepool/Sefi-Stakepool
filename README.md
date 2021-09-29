# TESTNET

`* secret1xj3v5q0q0p5khvark9hmcjrdxrjgh9zqsq7lhg`

# Sefi Stakepool

```

* (DEPOSIT): User can deposit Sefi.
      * Minimum Deposit 1 Sefi or 1000000 uSefi. 
            * Frontend also limits the deposit of lesser than 1 Sefi or 1000000 uSefi.
      * Maximum - No limit.
      * Deposits are refundable and can be canceled once bonded.
      * Deposits can be done anytime.
            * User with deposits at the 
                  * start of the Stakepool interval gets maximum weightage for the lottery.
                  * end of the Stakepool interval gets minimum weightage for the lottery.
            * Weightage of the deposits decreases as the Stakepool period comes to an end.


* (STAKING): Deposits are directly staked on the SEFI stake contract
      * When users buy tickets, the contract receive their tokens and deposit them on the staking contract.
      * Rewards are collected any time there is a deposit.
      * So the next deposit will also have those rewards collected on the previous deposit.
      * Rewards collected as restaked with every deposit.
      
* (REDUCE STAKES / TRIGGER WITHDRAW):
      *  
      

