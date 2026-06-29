# Cross-Chain Liquidation Hook

## Overview
This module enables liquidation triggered from bridge attestations.

## Flow
1. Bridge sends attestation
2. Contract verifies:
   - Admin authorization
   - Signature validity
   - Replay protection (nonce tracking)
3. If valid → triggers local liquidation

## Security Model
- Only admin can call hook
- Nonces prevent replay attacks
- Signature validation required (stubbed in current version)

## Future Improvements
- Replace stub signature check with Ed25519 or Secp256k1
- Add Merkle proof verification for bridge messages
- Gas optimization for nonce storage