
## [0.1.0] - 2026-10-28

### ⛰️  Features

- *(a11y)* Project widget semantics onto AccessKit - ([88550c4](https://github.com/QaidVoid/smudge/commit/88550c45fef6ba83b06b7ab613c160052afe8c17))
- *(core)* Paint in scroll-relative coordinates - ([6b565a4](https://github.com/QaidVoid/smudge/commit/6b565a4cc8f24e16b14487a5203c62835f3883be))
- *(core)* Add theme tokens resolved at paint time - ([9b392ee](https://github.com/QaidVoid/smudge/commit/9b392ee6d3818ea438d2bab4670576227e0792ab))
- *(cpu)* Draw glyph runs from a bounded mask cache - ([c3cad5e](https://github.com/QaidVoid/smudge/commit/c3cad5ebb4af3fa474851324d9198f51a5b8c683))
- *(text)* Add an opt-in feature for break dictionaries - ([dc9f444](https://github.com/QaidVoid/smudge/commit/dc9f4440f67647d8da0893b9a775df3eb448fb15))
- *(text)* Add undo and redo to the text input - ([bfcfc60](https://github.com/QaidVoid/smudge/commit/bfcfc609aad9a01b0001706d263c171cd6e630e2))
- *(text)* Add the text input widget - ([ef3d6a1](https://github.com/QaidVoid/smudge/commit/ef3d6a1c4d541b9e88fd450b46a2b6236ac9bd94))
- *(text)* Add the label widget - ([1ad9a09](https://github.com/QaidVoid/smudge/commit/1ad9a092dda33fb644c631f8c650b387cc0309b7))
- *(text)* Add the text system with bundled fonts - ([18f4e3a](https://github.com/QaidVoid/smudge/commit/18f4e3a37879afa1f51564ce6db33473eefb3c6a))
- Add IME composing and clipboard keys to text inputs - ([15473b6](https://github.com/QaidVoid/smudge/commit/15473b65655f7bfe7006108c9c897d62d1878302))

### 🐛 Bug Fixes

- *(core)* Wake no frame for writes the frame applies - ([a29caf8](https://github.com/QaidVoid/smudge/commit/a29caf8dc42d8c1386efafb0abe4f224adef3036))

### 📚 Documentation

- Align paint coordinate docs with scroll-relative painting - ([6573737](https://github.com/QaidVoid/smudge/commit/657373784a4d52abbc13871219d644b4c82b7a5c))

### ⚡ Performance

- *(text)* Shape a label once and re-break it per width - ([f5b1a17](https://github.com/QaidVoid/smudge/commit/f5b1a174da42bb83c44ff35c5c26bc67d8e823e9))
