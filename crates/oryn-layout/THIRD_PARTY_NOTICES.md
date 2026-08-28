# Oryn Layout Third-Party Notices

- Noto Sans Regular is pinned from `notofonts/noto-fonts` commit
  `ffebf8c1ee449e544955a7e813c54f9b73848eac`, file
  `hinted/ttf/NotoSans/NotoSans-Regular.ttf`. It is distributed under the
  SIL Open Font License 1.1; the complete license is in `assets/OFL.txt`.
- Taffy, Parley, Skrifa, and Wuff retain their upstream license terms as
  recorded in `Cargo.lock` and their published crate packages.
- Stylo is used under MPL-2.0. `stylo_taffy` is pinned from Blitz commit
  `9d5cef2fa3d01423081681843d91b76ef593443e` and retains its
  MIT/Apache-2.0/MPL-2.0 terms. Oryn's adapter is independently implemented
  against Oryn's read-only layout snapshot.

Oryn does not discover or load system fonts. Web fonts are accepted only
through the parent resource broker and are subject to page font limits.
