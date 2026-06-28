Provider logos for the Usage screen.

Drop a square PNG (transparent background, ~64–128px) for any provider here, named
by the provider key. The usage card shows the logo when present, otherwise it draws
a colored monogram chip in the provider's brand color.

Expected filenames:
  claude.png      -> Claude
  codex.png       -> Codex
  glm.png         -> GLM Coding Plan (z.ai)
  opencode.png    -> opencode

Notes:
  - Filenames match Provider::label() in crates/petcore/src/providers/mod.rs.
  - PNG decoding is already enabled in petgui; no rebuild flags needed.
  - Loaded once at startup (crates/petgui/src/logos.rs), so restart cc-pet after
    adding or changing a logo.
  - You can also point CC_PET_ASSETS at a different assets dir.
