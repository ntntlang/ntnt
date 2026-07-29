## 10. Larrimon definition of done

This consumer adoption is complete when:

- all 27 audited scenarios and 38 assertion/outcome lines are verified, corrected, superseded, or explicitly documentation-only; none vanish silently;
- every shell, Python, JavaScript test, and SQL-only application-test invariant has a destination and same-revision parity evidence;
- representative semantic mutations/faults prove detection before each old file is deleted;
- `tests/intent.sh` and suite wrappers are removed;
- project-owned `.sh` and `.py` support/orchestration files are zero, except operator-locked externally owned non-support artifacts;
- project-local browser/reconciliation test `.js`/`.mjs` and SQL-only application-test files are zero;
- typed project-state and `ntnt project env` replace dev/staging lifecycle programs with allocation, failure, cleanup, and mutation parity;
- the baseline inventory, protected contract, candidate base, execution snapshot, and evidence bind the same exact Larrimon commit and canonical digest;
- fast and full profiles run through ntnt with current verified coverage at the configured threshold;
- specialist external resources remain pinned, capability-scoped, and visible in reports;
- the complete old-to-new invariant ledger and mutation/fault witnesses remain in project history.

Expected end-state commands:

```bash
ntnt intent lint .
ntnt intent plan . --profile full --json
ntnt intent check . --profile fast
ntnt intent check . --profile full --report-json verification-report.json
```

Environment-backed protected profiles remain operator-selected outside the checkout.
