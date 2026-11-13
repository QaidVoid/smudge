
## [0.2.0](https://github.com/QaidVoid/smudge/compare/smudge-gpu-v0.1.0...smudge-gpu-v0.2.0) - 2026-11-13

### ⛰️  Features

- *(widgets)* Add the image widget - ([f927256](https://github.com/QaidVoid/smudge/commit/f92725662b5dacbd891a0f2f5facf432a57fe628))
- [**breaking**] Add premultiplied images to both renderers - ([8487574](https://github.com/QaidVoid/smudge/commit/8487574d865f84b0915f844e4eaf06339082713a))

### 🐛 Bug Fixes

- *(core)* [**breaking**] Harden focus, channels, cursors and bindings - ([5225d10](https://github.com/QaidVoid/smudge/commit/5225d10210257a22b8bd433b979942f5857fe437))
- *(gpu)* Keep every image drawn and retry other backends - ([72f3057](https://github.com/QaidVoid/smudge/commit/72f3057561e50ecfdacb7bc3095971dc68a0dcea))
- *(gpu)* Downscale images larger than the GPU atlas - ([05749bd](https://github.com/QaidVoid/smudge/commit/05749bdd406c9cc6c7991e3ad993477cf2a9f09f))
- *(gpu)* Request the adapter maximum storage binding limit - ([efd2ca3](https://github.com/QaidVoid/smudge/commit/efd2ca3a56a71691746f05a628b92d2ecf71f950))

### ⚡ Performance

- *(gpu)* Compile area antialiasing only and skip OpenGL - ([4f141a2](https://github.com/QaidVoid/smudge/commit/4f141a22d6a7bc8c06803ae5ec50e95c7e0e0225))

## [0.1.0] - 2026-10-28

### ⛰️  Features

- *(bench)* Record the GPU adapter and machine load in reports - ([b74177e](https://github.com/QaidVoid/smudge/commit/b74177e68c19e293831acbead2ac3b4baac67961))
- *(bench)* Add measurement scenes and a renderer choice - ([007baf6](https://github.com/QaidVoid/smudge/commit/007baf6380c8938813c3cfa07166d45f63ca9b1f))
- *(gpu)* Add a surface presenter for wgpu windows - ([5d3d744](https://github.com/QaidVoid/smudge/commit/5d3d744b52d04e23b074989fd17218a256e2a189))
- *(gpu)* Add the renderer trait and a vello renderer - ([cc81c57](https://github.com/QaidVoid/smudge/commit/cc81c57a9a2ecd5ec3bfc9f23d2f1f8ba635b847))
- *(paint)* Add offset display items that both renderers honor - ([37c2db9](https://github.com/QaidVoid/smudge/commit/37c2db9015e03e68e24c27811991dfdbbe0c832c))

### 🐛 Bug Fixes

- *(gpu)* Draw borders as rings and add a renderer parity test - ([a2899b9](https://github.com/QaidVoid/smudge/commit/a2899b9b1d71b8aa1cf3722bd067904cee902bc9))
