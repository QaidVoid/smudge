
## [0.2.0](https://github.com/QaidVoid/smudge/compare/v0.1.0...v0.2.0) - 2026-11-13

### ⛰️  Features

- *(core)* [**breaking**] Make scroll views scroll containers in layout - ([92c6031](https://github.com/QaidVoid/smudge/commit/92c603147c2b02ed4ba417a54a39bacb5a9f2aaa))
- *(core)* Pin a flowing child in place - ([de6a6bd](https://github.com/QaidVoid/smudge/commit/de6a6bd7359cd4f283731dbcf8876067e5ee4bbc))
- *(core)* [**breaking**] Follow a scroll view to its end - ([ac09643](https://github.com/QaidVoid/smudge/commit/ac0964363145cbe70c91e89a6762c235b18de602))
- *(core)* Add root keys, autofocus and focus traps - ([6a84e85](https://github.com/QaidVoid/smudge/commit/6a84e85aabece89bfc3888127fcfc31d8692c22f))
- *(core)* Add window commands, state and cursors - ([c584c1f](https://github.com/QaidVoid/smudge/commit/c584c1f40cca783122a6dbaf72c953e82fb1795d))
- *(core)* Add channels from other threads - ([e5e15cd](https://github.com/QaidVoid/smudge/commit/e5e15cdd9f6db9901ca78ea31b38e2372be8d82a))
- *(platform)* [**breaking**] Add window options, commands and cursors - ([8be803e](https://github.com/QaidVoid/smudge/commit/8be803e9766ba90c9595a0bbaf248e9853894571))
- *(platform)* Wake the window when a message arrives - ([c3f7d91](https://github.com/QaidVoid/smudge/commit/c3f7d91fc5bdea7c07afb12e34dc4b04e2f6040a))
- *(smudge)* Export image from the prelude - ([ae45689](https://github.com/QaidVoid/smudge/commit/ae45689644e2333a7073eecab76162863da0ffd1))
- *(smudge)* Forward system-fonts and font types - ([d68a539](https://github.com/QaidVoid/smudge/commit/d68a53911784723fea6d513ad147f030d6634e8d))
- *(text)* [**breaking**] Add the multi-line text area - ([ef2416e](https://github.com/QaidVoid/smudge/commit/ef2416e57ff2691f996739c3049e16080eac772b))
- *(text)* [**breaking**] Cap label lines with an ellipsis - ([e98c012](https://github.com/QaidVoid/smudge/commit/e98c01235f6c3cb9a569755c3ea3970fe58f90c6))
- *(text)* Fall SystemUi back to the bundled sans - ([9fc52c9](https://github.com/QaidVoid/smudge/commit/9fc52c99df2e2b33170206b5341efeffb4f02525))
- *(text)* Re-export the font style types - ([9ff7aa5](https://github.com/QaidVoid/smudge/commit/9ff7aa5d4fccccaa46fc71c513aa476af85d0079))
- *(text)* Show the text cursor over a text input - ([30de236](https://github.com/QaidVoid/smudge/commit/30de236d474abb6010f718eef758e80721eae760))
- *(widgets)* Add the image widget - ([f927256](https://github.com/QaidVoid/smudge/commit/f92725662b5dacbd891a0f2f5facf432a57fe628))
- *(widgets)* Add a keyed virtual list with an anchor - ([e8d624e](https://github.com/QaidVoid/smudge/commit/e8d624ee8eca4a2826736dce858d9da87f2b7fb9))
- [**breaking**] Add premultiplied images to both renderers - ([8487574](https://github.com/QaidVoid/smudge/commit/8487574d865f84b0915f844e4eaf06339082713a))

### 🐛 Bug Fixes

- *(core)* [**breaking**] Harden focus, channels, cursors and bindings - ([5225d10](https://github.com/QaidVoid/smudge/commit/5225d10210257a22b8bd433b979942f5857fe437))
- *(core)* Let the last builder call for a property win - ([248b2c9](https://github.com/QaidVoid/smudge/commit/248b2c910fda4b695039d5e1b7df11974289cba8))
- *(examples)* Match the window theme to the dark clear color - ([e61e188](https://github.com/QaidVoid/smudge/commit/e61e188280324ba318a146cff5312959e589766d))
- *(gpu)* Keep every image drawn and retry other backends - ([72f3057](https://github.com/QaidVoid/smudge/commit/72f3057561e50ecfdacb7bc3095971dc68a0dcea))
- *(gpu)* Downscale images larger than the GPU atlas - ([05749bd](https://github.com/QaidVoid/smudge/commit/05749bdd406c9cc6c7991e3ad993477cf2a9f09f))
- *(gpu)* Request the adapter maximum storage binding limit - ([efd2ca3](https://github.com/QaidVoid/smudge/commit/efd2ca3a56a71691746f05a628b92d2ecf71f950))
- *(platform)* Pace frames where the window system does not - ([f0d327d](https://github.com/QaidVoid/smudge/commit/f0d327dd0487322e8809d628dda0820be5e0a221))
- *(platform)* End the press when the window system takes it - ([7a36fc9](https://github.com/QaidVoid/smudge/commit/7a36fc926f7fd072bf895ba7bae860473e108355))
- *(text)* [**breaking**] Correct editing edges and let the wheel pass on - ([7a34069](https://github.com/QaidVoid/smudge/commit/7a34069832dd897eb8a5356505f83529a81aa307))
- *(text)* Flatten pasted line breaks into single spaces - ([f58bff3](https://github.com/QaidVoid/smudge/commit/f58bff3c9ea83a9eef12fc8081a03b5cf1c6b01d))
- *(widgets)* [**breaking**] Correct keyed list edges and image measuring - ([2f0e017](https://github.com/QaidVoid/smudge/commit/2f0e017bd5ebf12bcc1d35156899495b85fa80f3))

### 📚 Documentation

- Correct the guide for 0.2 and replay the recorded seeds - ([327c7bc](https://github.com/QaidVoid/smudge/commit/327c7bc375a11e259ae072d2fad99f69c66e8ead))

### ⚡ Performance

- *(gpu)* Compile area antialiasing only and skip OpenGL - ([4f141a2](https://github.com/QaidVoid/smudge/commit/4f141a22d6a7bc8c06803ae5ec50e95c7e0e0225))

## [0.1.0] - 2026-10-28

### ⛰️  Features

- *(a11y)* Project widget semantics onto AccessKit - ([88550c4](https://github.com/QaidVoid/smudge/commit/88550c45fef6ba83b06b7ab613c160052afe8c17))
- *(bench)* Add measurement scenes and a renderer choice - ([007baf6](https://github.com/QaidVoid/smudge/commit/007baf6380c8938813c3cfa07166d45f63ca9b1f))
- *(bench)* Add the scene suite and measurement reports - ([d552aac](https://github.com/QaidVoid/smudge/commit/d552aac3286a909325b3afa56229bf7dab7f60e6))
- *(bench)* Record the GPU adapter and machine load in reports - ([b74177e](https://github.com/QaidVoid/smudge/commit/b74177e68c19e293831acbead2ac3b4baac67961))
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
- *(cpu)* Add the CPU renderer that repaints damage - ([29ea377](https://github.com/QaidVoid/smudge/commit/29ea37731359a1e9e372ce794ebfdcf4b93667e6))
- *(gpu)* Add the renderer trait and a vello renderer - ([cc81c57](https://github.com/QaidVoid/smudge/commit/cc81c57a9a2ecd5ec3bfc9f23d2f1f8ba635b847))
- *(gpu)* Add a surface presenter for wgpu windows - ([5d3d744](https://github.com/QaidVoid/smudge/commit/5d3d744b52d04e23b074989fd17218a256e2a189))
- *(paint)* Add offset display items that both renderers honor - ([37c2db9](https://github.com/QaidVoid/smudge/commit/37c2db9015e03e68e24c27811991dfdbbe0c832c))
- *(platform)* Run the CPU renderer in a browser - ([65d1895](https://github.com/QaidVoid/smudge/commit/65d1895fa2fb6eb12f78a7f2d4887411fdfe8181))
- *(platform)* Choose between the CPU and GPU renderer - ([8733b9d](https://github.com/QaidVoid/smudge/commit/8733b9d0da1ccaf3f750a8152d68b0072d990dbf))
- *(platform)* Add a damage overlay for window debugging - ([14b0ed9](https://github.com/QaidVoid/smudge/commit/14b0ed9910c4257038443c893e5ceff6929c6bb8))
- *(platform)* Skip presents that change nothing - ([b6d74a7](https://github.com/QaidVoid/smudge/commit/b6d74a787b850380a97436621148a7bbbd056bd4))
- *(platform)* Present only damage when the buffer age allows - ([d8fb2b5](https://github.com/QaidVoid/smudge/commit/d8fb2b536b0105618fcf2c290f42681a922d1130))
- *(platform)* Add the window and run loop - ([a7a4597](https://github.com/QaidVoid/smudge/commit/a7a45974388bc96bf21a495c6e2a8a98459b1c09))
- *(reactive)* Run signals on the in-house runtime - ([ca36f33](https://github.com/QaidVoid/smudge/commit/ca36f33f7eeed93b52f7078373c9e49fce810d95))
- *(smudge)* Add the facade crate, examples and docs - ([6c4c936](https://github.com/QaidVoid/smudge/commit/6c4c936cf57396dee23e056bf903528406af3896))
- *(test)* Add the headless harness for frames and damage - ([9336ff3](https://github.com/QaidVoid/smudge/commit/9336ff31aaafbea8f1db99a40d8b4141376b727e))
- *(text)* Add an opt-in feature for break dictionaries - ([dc9f444](https://github.com/QaidVoid/smudge/commit/dc9f4440f67647d8da0893b9a775df3eb448fb15))
- *(text)* Add the label widget - ([1ad9a09](https://github.com/QaidVoid/smudge/commit/1ad9a092dda33fb644c631f8c650b387cc0309b7))
- *(text)* Add undo and redo to the text input - ([bfcfc60](https://github.com/QaidVoid/smudge/commit/bfcfc609aad9a01b0001706d263c171cd6e630e2))
- *(text)* Add the text input widget - ([ef3d6a1](https://github.com/QaidVoid/smudge/commit/ef3d6a1c4d541b9e88fd450b46a2b6236ac9bd94))
- *(text)* Add the text system with bundled fonts - ([18f4e3a](https://github.com/QaidVoid/smudge/commit/18f4e3a37879afa1f51564ce6db33473eefb3c6a))
- *(widgets)* Add a virtual list with fixed row height - ([b82a9e2](https://github.com/QaidVoid/smudge/commit/b82a9e25240c13d9fbaf54b1bc60aa32e6dd5dad))
- *(widgets)* Add the scroll view - ([014f3af](https://github.com/QaidVoid/smudge/commit/014f3afc73749b67257b68b95261d1408b70f389))
- *(widgets)* Add containers and button - ([38fb5c0](https://github.com/QaidVoid/smudge/commit/38fb5c0f58ab52bf9867dbcce09163594a88e3c0))
- Run due timers without drawing a frame - ([28e7bc9](https://github.com/QaidVoid/smudge/commit/28e7bc9977ff44ce9255cc43e79fd2d747e4d2f3))
- Add platform-independent input events - ([2ecb476](https://github.com/QaidVoid/smudge/commit/2ecb4764a2d65b3bce3ca66574ef3f4d88fafbb1))
- Add IME composing and clipboard keys to text inputs - ([15473b6](https://github.com/QaidVoid/smudge/commit/15473b65655f7bfe7006108c9c897d62d1878302))

### 🐛 Bug Fixes

- *(core)* End a replaced pointer capture with its release - ([7efc7f7](https://github.com/QaidVoid/smudge/commit/7efc7f7ba907631087af3bbd18f2369f844910a2))
- *(core)* Wake no frame for writes the frame applies - ([a29caf8](https://github.com/QaidVoid/smudge/commit/a29caf8dc42d8c1386efafb0abe4f224adef3036))
- *(core)* Keep layout working after removing an absolute child - ([3dfd8a4](https://github.com/QaidVoid/smudge/commit/3dfd8a41ec1b62014e119f0d2e68e011baa28ebd))
- *(core)* Damage the pixels a clip edge passes through - ([a452807](https://github.com/QaidVoid/smudge/commit/a452807c8fefa4aafb5b0004583c839f3cd7bb4a))
- *(gpu)* Draw borders as rings and add a renderer parity test - ([a2899b9](https://github.com/QaidVoid/smudge/commit/a2899b9b1d71b8aa1cf3722bd067904cee902bc9))
- *(platform)* Keep drawing until a frame limit is reached - ([0a3d0d5](https://github.com/QaidVoid/smudge/commit/0a3d0d57737eaf5087046fcbea2257c6be01a88d))
- *(platform)* Report wheel deltas positive for scrolling down - ([2310d3a](https://github.com/QaidVoid/smudge/commit/2310d3a6be543068afcbc774e1a840c02599cff2))
- *(platform)* Time the wait for a window buffer on its own - ([5446370](https://github.com/QaidVoid/smudge/commit/544637078a03f7ab20aa0684313536b4e36093f9))
- *(platform)* Copy damage repeated across frames once - ([b647c78](https://github.com/QaidVoid/smudge/commit/b647c787047760ebde6e1b486391f75f944f35be))

### 🚜 Refactor

- *(core)* Define measure input types in smudge-core - ([6396d87](https://github.com/QaidVoid/smudge/commit/6396d8735c3584dbbb9af20b1f0794cfb44fcd19))
- *(core)* Hide the damage module and drop unused helpers - ([f956725](https://github.com/QaidVoid/smudge/commit/f9567259ff2b3d1e640854ab3739b8c9e1247d1f))

### 📚 Documentation

- Align paint coordinate docs with scroll-relative painting - ([6573737](https://github.com/QaidVoid/smudge/commit/657373784a4d52abbc13871219d644b4c82b7a5c))

### ⚡ Performance

- *(core)* Collapse scattered damage past 64 rects to the surface - ([fabb7cd](https://github.com/QaidVoid/smudge/commit/fabb7cd4116936d9f183e1c6abf933f57b98fbc0))
- *(core)* Add unchanged painted bounds to the damage once - ([e231e07](https://github.com/QaidVoid/smudge/commit/e231e07c4520668baac29585b1a7ce9d81b3fe60))
- *(cpu)* Skip offset translation when the offset is zero - ([6328653](https://github.com/QaidVoid/smudge/commit/6328653fe04344217a1299ca7bced968fdeb1311))
- *(cpu)* Cull the raster across damage rects - ([3358f00](https://github.com/QaidVoid/smudge/commit/3358f004b7a8d15ea0544a191d4239ee21d7c1d9))
- *(text)* Shape a label once and re-break it per width - ([f5b1a17](https://github.com/QaidVoid/smudge/commit/f5b1a174da42bb83c44ff35c5c26bc67d8e823e9))

### ⚙️ Miscellaneous Tasks

- Set up cargo workspace and crate skeletons - ([e0ead88](https://github.com/QaidVoid/smudge/commit/e0ead88292cadc8dd6bb09548ba3405eb01784a0))

### Build

- *(core)* Drop reactive_graph and the futures stack - ([93271ee](https://github.com/QaidVoid/smudge/commit/93271eece36c04f24b3bf58317ab94cfcb82d4f2))
