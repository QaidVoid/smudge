
## [0.1.0] - 2026-10-28

### ⛰️  Features

- *(a11y)* Project widget semantics onto AccessKit - ([88550c4](https://github.com/QaidVoid/smudge/commit/88550c45fef6ba83b06b7ab613c160052afe8c17))
- *(bench)* Record the GPU adapter and machine load in reports - ([b74177e](https://github.com/QaidVoid/smudge/commit/b74177e68c19e293831acbead2ac3b4baac67961))
- *(bench)* Add measurement scenes and a renderer choice - ([007baf6](https://github.com/QaidVoid/smudge/commit/007baf6380c8938813c3cfa07166d45f63ca9b1f))
- *(bench)* Add the scene suite and measurement reports - ([d552aac](https://github.com/QaidVoid/smudge/commit/d552aac3286a909325b3afa56229bf7dab7f60e6))
- *(core)* Add timers and pointer and IME routing - ([b7c4b17](https://github.com/QaidVoid/smudge/commit/b7c4b17196940e89a23b65352e8d12acd3109a8f))
- *(core)* Add theme tokens resolved at paint time - ([9b392ee](https://github.com/QaidVoid/smudge/commit/9b392ee6d3818ea438d2bab4670576227e0792ab))
- *(core)* Add style variants resolved in a fixed order - ([7b66625](https://github.com/QaidVoid/smudge/commit/7b66625251e486e62b401e73287818a77128c24c))
- *(platform)* Run the CPU renderer in a browser - ([65d1895](https://github.com/QaidVoid/smudge/commit/65d1895fa2fb6eb12f78a7f2d4887411fdfe8181))
- *(platform)* Choose between the CPU and GPU renderer - ([8733b9d](https://github.com/QaidVoid/smudge/commit/8733b9d0da1ccaf3f750a8152d68b0072d990dbf))
- *(platform)* Add a damage overlay for window debugging - ([14b0ed9](https://github.com/QaidVoid/smudge/commit/14b0ed9910c4257038443c893e5ceff6929c6bb8))
- *(platform)* Skip presents that change nothing - ([b6d74a7](https://github.com/QaidVoid/smudge/commit/b6d74a787b850380a97436621148a7bbbd056bd4))
- *(platform)* Present only damage when the buffer age allows - ([d8fb2b5](https://github.com/QaidVoid/smudge/commit/d8fb2b536b0105618fcf2c290f42681a922d1130))
- *(platform)* Add the window and run loop - ([a7a4597](https://github.com/QaidVoid/smudge/commit/a7a45974388bc96bf21a495c6e2a8a98459b1c09))
- *(reactive)* Run signals on the in-house runtime - ([ca36f33](https://github.com/QaidVoid/smudge/commit/ca36f33f7eeed93b52f7078373c9e49fce810d95))
- Add IME composing and clipboard keys to text inputs - ([15473b6](https://github.com/QaidVoid/smudge/commit/15473b65655f7bfe7006108c9c897d62d1878302))
- Run due timers without drawing a frame - ([28e7bc9](https://github.com/QaidVoid/smudge/commit/28e7bc9977ff44ce9255cc43e79fd2d747e4d2f3))
- Add platform-independent input events - ([2ecb476](https://github.com/QaidVoid/smudge/commit/2ecb4764a2d65b3bce3ca66574ef3f4d88fafbb1))

### 🐛 Bug Fixes

- *(platform)* Keep drawing until a frame limit is reached - ([0a3d0d5](https://github.com/QaidVoid/smudge/commit/0a3d0d57737eaf5087046fcbea2257c6be01a88d))
- *(platform)* Report wheel deltas positive for scrolling down - ([2310d3a](https://github.com/QaidVoid/smudge/commit/2310d3a6be543068afcbc774e1a840c02599cff2))
- *(platform)* Time the wait for a window buffer on its own - ([5446370](https://github.com/QaidVoid/smudge/commit/544637078a03f7ab20aa0684313536b4e36093f9))
- *(platform)* Copy damage repeated across frames once - ([b647c78](https://github.com/QaidVoid/smudge/commit/b647c787047760ebde6e1b486391f75f944f35be))

### ⚙️ Miscellaneous Tasks

- Set up cargo workspace and crate skeletons - ([e0ead88](https://github.com/QaidVoid/smudge/commit/e0ead88292cadc8dd6bb09548ba3405eb01784a0))
