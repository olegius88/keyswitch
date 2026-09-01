# Frozen Onboard training sources

These files are the immutable lexical inputs used to train and reproduce
KeySwitch Layout Intent v1. Keeping the exact bytes in the repository prevents
the sealed dataset and model artifact from changing when a Linux distribution
updates `onboard-data`.

Source package installed on the training host:

- package: `onboard-data`;
- version: `1.4.3+git20260213+ds-2`;
- distribution: Ubuntu 26.04;
- `en_US.lm` SHA-256:
  `017a513a23bb37947a3a0e73e307066fa8ec0642075172cdcad6ea204cae68da`;
- `ru_RU.lm` SHA-256:
  `e57c14eec2b78e52a2125dce2b38044d8dfe480f3964b9d118614527195ceda9`;
- `COPYRIGHT.onboard-data` SHA-256:
  `bafb01a0e2cbdf8a2717cd3678b94329aa11ec8963c88f9ad2ceb1e7473d8033`.

`COPYRIGHT.onboard-data` is copied byte-for-byte from
`/usr/share/doc/onboard-data/copyright`. Its `Files: models/*` stanza is the
authoritative license declaration for the two `.lm` files. This README records
provenance only and does not replace or reinterpret that declaration.
