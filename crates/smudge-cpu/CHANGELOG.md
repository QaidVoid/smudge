
## [0.1.0] - 2026-10-28

### ⛰️  Features

- *(core)* Paint in scroll-relative coordinates - ([6b565a4](https://github.com/QaidVoid/smudge/commit/6b565a4cc8f24e16b14487a5203c62835f3883be))
- *(core)* Add theme tokens resolved at paint time - ([9b392ee](https://github.com/QaidVoid/smudge/commit/9b392ee6d3818ea438d2bab4670576227e0792ab))
- *(cpu)* Draw glyph runs from a bounded mask cache - ([c3cad5e](https://github.com/QaidVoid/smudge/commit/c3cad5ebb4af3fa474851324d9198f51a5b8c683))
- *(cpu)* Add the CPU renderer that repaints damage - ([29ea377](https://github.com/QaidVoid/smudge/commit/29ea37731359a1e9e372ce794ebfdcf4b93667e6))
- *(gpu)* Add the renderer trait and a vello renderer - ([cc81c57](https://github.com/QaidVoid/smudge/commit/cc81c57a9a2ecd5ec3bfc9f23d2f1f8ba635b847))
- *(paint)* Add offset display items that both renderers honor - ([37c2db9](https://github.com/QaidVoid/smudge/commit/37c2db9015e03e68e24c27811991dfdbbe0c832c))

### ⚡ Performance

- *(cpu)* Skip offset translation when the offset is zero - ([6328653](https://github.com/QaidVoid/smudge/commit/6328653fe04344217a1299ca7bced968fdeb1311))
- *(cpu)* Cull the raster across damage rects - ([3358f00](https://github.com/QaidVoid/smudge/commit/3358f004b7a8d15ea0544a191d4239ee21d7c1d9))

### ⚙️ Miscellaneous Tasks

- Set up cargo workspace and crate skeletons - ([e0ead88](https://github.com/QaidVoid/smudge/commit/e0ead88292cadc8dd6bb09548ba3405eb01784a0))
