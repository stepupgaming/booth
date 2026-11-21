# What Booth changed in oxideav-aac 0.1.7

The crates.io release encodes about 0.6x real time on a fast desktop, too slow for a long
voiceover on a phone. Two changes make it about 50x real time. The bitstream it writes is the
same, bit for bit.

1. `src/filterbank.rs`: `forward_mdct` uses an FFT (fold to a DCT-IV, N/4-point complex FFT)
   instead of the direct O(N^2) sum. The direct form is kept as `forward_mdct_naive` and is used
   when N/2 is not a power of two. The test `fast_mdct_matches_the_direct_form` compares them.
2. `src/encoder.rs`: `band_bits` counts bits from the codeword length tables instead of writing
   every tuple into a scratch `BitWriter` (which allocated several times per tuple). The old
   version is kept as `band_bits_slow` under `#[cfg(test)]`, and `band_bits_matches_slow_writer`
   compares them on 20,000 random bands.

All 897 library tests and the upstream integration suites passed with both changes.
