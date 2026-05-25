# TERRA — Verifiable Asset Tokenization Protocol

Precision financial infrastructure for African Real-World Assets (RWAs).

## Overview

TERRA enables farmers, SMEs, and communities to tokenize real assets (crops, land, herbal medicine, commodities) on Solana with verifiable proof, precise yield calculations, and transparent investor access.

### Three Core Components

- **terra-vault**: On-chain settlement engine. Deposits, withdrawals, yield calculations, tranche management.
- **terra-attestation**: Verifiable asset layer. Agents sign asset claims. Merkle tree batching for cheap on-chain verification.
- **terra-marketplace**: Tranche trading. Investors buy/sell yield exposure. MEV-resistant batch auctions.

## Phase 1: Precision Settlement Backbone

Building the core vault program with Safe Vault precision math deployed on Solana.

## Getting Started

```bash
# Install Rust (if not already done)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Anchor
cargo install --git https://github.com/coral-xyz/anchor anchor-cli

# Build programs
anchor build

# Run tests
anchor test
