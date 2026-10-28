
## [0.1.0] - 2026-10-28

### ⛰️  Features

- *(a11y)* Project widget semantics onto AccessKit - ([88550c4](https://github.com/QaidVoid/smudge/commit/88550c45fef6ba83b06b7ab613c160052afe8c17))
- *(bench)* Add the scene suite and measurement reports - ([d552aac](https://github.com/QaidVoid/smudge/commit/d552aac3286a909325b3afa56229bf7dab7f60e6))
- *(core)* Paint in scroll-relative coordinates - ([6b565a4](https://github.com/QaidVoid/smudge/commit/6b565a4cc8f24e16b14487a5203c62835f3883be))
- *(core)* Add the pct length builder - ([6392806](https://github.com/QaidVoid/smudge/commit/6392806dbd7bc31006de343d11900e6a4b995db7))
- *(core)* Add a clipboard and IME preedit routing - ([76dd32d](https://github.com/QaidVoid/smudge/commit/76dd32dcb25049955674e332d35daaec78a8d9b6))
- *(core)* Add timers and pointer and IME routing - ([b7c4b17](https://github.com/QaidVoid/smudge/commit/b7c4b17196940e89a23b65352e8d12acd3109a8f))
- *(core)* Add scroll offsets and wheel routing - ([030474f](https://github.com/QaidVoid/smudge/commit/030474f47cde85f009179094cd6e1e85c65db820))
- *(core)* Add keyed dynamic children - ([150323d](https://github.com/QaidVoid/smudge/commit/150323d02de0725e55b1c8b7776515ba45eb24c4))
- *(core)* Add child styling, key activation and accent tokens - ([b903479](https://github.com/QaidVoid/smudge/commit/b90347930e5d0485a3a1da6557d2617f19ad5332))
- *(core)* Add theme tokens resolved at paint time - ([9b392ee](https://github.com/QaidVoid/smudge/commit/9b392ee6d3818ea438d2bab4670576227e0792ab))
- *(core)* Add style variants resolved in a fixed order - ([7b66625](https://github.com/QaidVoid/smudge/commit/7b66625251e486e62b401e73287818a77128c24c))
- *(core)* Add keyboard focus and key routing - ([40751b2](https://github.com/QaidVoid/smudge/commit/40751b269f920fa223a5b80227f59ed896f503f3))
- *(core)* Add hit testing, hover and press routing - ([d6d934b](https://github.com/QaidVoid/smudge/commit/d6d934b36aa132c5c00a367877256f2f0408489d))
- *(core)* Add the app and the frame loop - ([e43b2c4](https://github.com/QaidVoid/smudge/commit/e43b2c454bf925a743f081b9b99209a65855b87b))
- *(core)* Add the paint pass - ([29d0647](https://github.com/QaidVoid/smudge/commit/29d0647b0c160689dd4f53809d3ac8214b3d515e))
- *(core)* Add damage accumulation and merging - ([68c799d](https://github.com/QaidVoid/smudge/commit/68c799da5e3873c1526e3fef308c18edd1cb44e3))
- *(core)* Add the layout pass - ([990f85c](https://github.com/QaidVoid/smudge/commit/990f85c63180f7b83d1b195f738e3c32ca707645))
- *(core)* Add elements, builders and reactive bindings - ([63e1dc9](https://github.com/QaidVoid/smudge/commit/63e1dc9189212501948de7bbad0ab702c47ea11a))
- *(core)* Add the widget tree arena - ([83017d4](https://github.com/QaidVoid/smudge/commit/83017d460515e7f510f869c354139e7a731bfb69))
- *(core)* Add style types and layout engine conversion - ([4505aa0](https://github.com/QaidVoid/smudge/commit/4505aa064a21daa51a56d5fc44106fb98d51d956))
- *(core)* Add the reactive API and the wake callback - ([813685f](https://github.com/QaidVoid/smudge/commit/813685f19c4298f35243c9389758901546080d77))
- *(core)* Run reactive effects on a frame-driven executor - ([1393512](https://github.com/QaidVoid/smudge/commit/1393512f1cc6a0abd709cac4f3e579edaeb24b4c))
- *(core)* Add paint operations and display list - ([2248828](https://github.com/QaidVoid/smudge/commit/2248828c236805674745ec139198e8b6ed4a08e6))
- *(core)* Add geometry and color types - ([a09fd8e](https://github.com/QaidVoid/smudge/commit/a09fd8eaa2fba7532df49bb31146118466d33c74))
- *(cpu)* Draw glyph runs from a bounded mask cache - ([c3cad5e](https://github.com/QaidVoid/smudge/commit/c3cad5ebb4af3fa474851324d9198f51a5b8c683))
- *(gpu)* Add the renderer trait and a vello renderer - ([cc81c57](https://github.com/QaidVoid/smudge/commit/cc81c57a9a2ecd5ec3bfc9f23d2f1f8ba635b847))
- *(paint)* Add offset display items that both renderers honor - ([37c2db9](https://github.com/QaidVoid/smudge/commit/37c2db9015e03e68e24c27811991dfdbbe0c832c))
- *(reactive)* Run signals on the in-house runtime - ([ca36f33](https://github.com/QaidVoid/smudge/commit/ca36f33f7eeed93b52f7078373c9e49fce810d95))
- *(test)* Add the headless harness for frames and damage - ([9336ff3](https://github.com/QaidVoid/smudge/commit/9336ff31aaafbea8f1db99a40d8b4141376b727e))
- *(text)* Add the label widget - ([1ad9a09](https://github.com/QaidVoid/smudge/commit/1ad9a092dda33fb644c631f8c650b387cc0309b7))
- Run due timers without drawing a frame - ([28e7bc9](https://github.com/QaidVoid/smudge/commit/28e7bc9977ff44ce9255cc43e79fd2d747e4d2f3))
- Add platform-independent input events - ([2ecb476](https://github.com/QaidVoid/smudge/commit/2ecb4764a2d65b3bce3ca66574ef3f4d88fafbb1))

### 🐛 Bug Fixes

- *(core)* End a replaced pointer capture with its release - ([7efc7f7](https://github.com/QaidVoid/smudge/commit/7efc7f7ba907631087af3bbd18f2369f844910a2))
- *(core)* Wake no frame for writes the frame applies - ([a29caf8](https://github.com/QaidVoid/smudge/commit/a29caf8dc42d8c1386efafb0abe4f224adef3036))
- *(core)* Keep layout working after removing an absolute child - ([3dfd8a4](https://github.com/QaidVoid/smudge/commit/3dfd8a41ec1b62014e119f0d2e68e011baa28ebd))
- *(core)* Damage the pixels a clip edge passes through - ([a452807](https://github.com/QaidVoid/smudge/commit/a452807c8fefa4aafb5b0004583c839f3cd7bb4a))

### 🚜 Refactor

- *(core)* Define measure input types in smudge-core - ([6396d87](https://github.com/QaidVoid/smudge/commit/6396d8735c3584dbbb9af20b1f0794cfb44fcd19))
- *(core)* Hide the damage module and drop unused helpers - ([f956725](https://github.com/QaidVoid/smudge/commit/f9567259ff2b3d1e640854ab3739b8c9e1247d1f))

### 📚 Documentation

- Align paint coordinate docs with scroll-relative painting - ([6573737](https://github.com/QaidVoid/smudge/commit/657373784a4d52abbc13871219d644b4c82b7a5c))

### ⚡ Performance

- *(core)* Collapse scattered damage past 64 rects to the surface - ([fabb7cd](https://github.com/QaidVoid/smudge/commit/fabb7cd4116936d9f183e1c6abf933f57b98fbc0))
- *(core)* Add unchanged painted bounds to the damage once - ([e231e07](https://github.com/QaidVoid/smudge/commit/e231e07c4520668baac29585b1a7ce9d81b3fe60))

### ⚙️ Miscellaneous Tasks

- Set up cargo workspace and crate skeletons - ([e0ead88](https://github.com/QaidVoid/smudge/commit/e0ead88292cadc8dd6bb09548ba3405eb01784a0))

### Build

- *(core)* Drop reactive_graph and the futures stack - ([93271ee](https://github.com/QaidVoid/smudge/commit/93271eece36c04f24b3bf58317ab94cfcb82d4f2))
