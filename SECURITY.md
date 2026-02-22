# Security Policy

## Supported Versions

| Version | Supported |
|---|---|
| 1.0.13 (current) | Yes |
| < 1.0.13 | No |

## Reporting a Vulnerability

**DO NOT** open a public GitHub issue for security vulnerabilities.

### Responsible Disclosure

If you discover a security vulnerability in Unauthority (LOS), please report it responsibly:

1. **Email:** Send a detailed report to the project maintainers via GitHub private security advisory
2. **GitHub Security Advisory:** Use the [Security Advisories](https://github.com/monkey-king-code/unauthority-core/security/advisories/new) feature to create a private report

### What to Include

- Description of the vulnerability
- Steps to reproduce
- Potential impact assessment
- Suggested fix (if applicable)
- Your contact information for follow-up

### Response Timeline

| Stage | Timeline |
|---|---|
| Acknowledgment | Within 48 hours |
| Initial assessment | Within 7 days |
| Fix development | Depends on severity |
| Public disclosure | After fix is deployed |

### Severity Classification

| Severity | Description | Example |
|---|---|---|
| **Critical** | Funds at risk, consensus bypass | Double-spend, signature forgery, supply inflation |
| **High** | Network disruption, data integrity | Consensus deadlock, ledger corruption |
| **Medium** | Limited impact, workaround exists | DoS vector, information disclosure |
| **Low** | Minor, cosmetic, or theoretical | UI bug, non-sensitive data leak |

## Security Considerations

### Cryptography

- **Signatures:** CRYSTALS-Dilithium5 (NIST FIPS 204, 256-bit classical security)
- **Hashing:** SHA-3 (Keccak, FIPS 202)
- **Key Derivation:** BIP39 mnemonic → SHA-3 expansion → Dilithium5 keygen

### Consensus Safety

- **aBFT:** Tolerates up to f = (n-1)/3 Byzantine validators
- **Quorum:** 2f+1 votes required for finalization
- **Determinism:** All consensus math uses u128 integer arithmetic (zero floating-point)

### Network Privacy

- **Tor-only:** All traffic routed through Tor hidden services
- **No clearnet:** Validators have no public IP exposure
- **P2P encryption:** Noise Protocol (XX handshake) over Tor transport

### Known Limitations

- Tor network introduces 500ms–2s latency per request
- Dilithium5 signatures (~4.6 KB) and public keys (~2.5 KB) are larger than ECDSA equivalents
- Post-quantum key sizes increase storage requirements

## Bug Bounty

There is currently no formal bug bounty program. Critical vulnerability reporters will be credited in the CHANGELOG and may receive recognition from the community.

## License

AGPL-3.0 — See [LICENSE](LICENSE)
