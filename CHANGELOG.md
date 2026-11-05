# Changelog

All notable changes to glance are listed here, newest first.

## Unreleased

### Features

- resolve conflicts in gg add ([55f6828](https://github.com/alvgaona/glance/commit/55f6828ac35cbb94b0d6cc6a981786528a101b85))
- stage single lines of a hunk in gg add ([5206d66](https://github.com/alvgaona/glance/commit/5206d6615055aa4e17b2fa6fd120aed5127e0df8))
- commit and amend from gg add ([73a6f31](https://github.com/alvgaona/glance/commit/73a6f317c0ba316a344abfd22f1386e1984adb83))
- stage, unstage and discard single hunks in gg add ([0f063d2](https://github.com/alvgaona/glance/commit/0f063d22bec7b115ec6c6bc3fd86807be534fcf8))
- gg add staging screen ([da81dac](https://github.com/alvgaona/glance/commit/da81dac6695faa0e8574b15447e96b770b1fbc76))
- staging reads and writes for gg add ([79f8b81](https://github.com/alvgaona/glance/commit/79f8b81faa06f806a344c3e929fae70b19e8a6b0))
- gg remote ([4c759b0](https://github.com/alvgaona/glance/commit/4c759b0020c437412701fde85bab1c64f4a72b48))

### Fixes

- size dialogs and toasts to their wrapped text ([5c6821e](https://github.com/alvgaona/glance/commit/5c6821e0d0f057d733cdaa0a3c0e4816fed6e09e))
- stash picked files from a subfolder ([260c275](https://github.com/alvgaona/glance/commit/260c275a6cbaa0535f971a152c2ed9436ce53314))
- gg stash and gg wt crash in short terminals ([2ccf6f0](https://github.com/alvgaona/glance/commit/2ccf6f071a16ae907cf236cd1c54864a68a1301f))

### Refactoring

- shared changed-file lists in panels/changes ([5900c84](https://github.com/alvgaona/glance/commit/5900c845458cc2bd88778c0f96f31767907671ea))
- split the big screens by job ([ab96a68](https://github.com/alvgaona/glance/commit/ab96a687e888d0dc4ed3716ae1f3d40087404e95))
- move screens into tui/screens and the diff panel into tui/panels ([cec7e3c](https://github.com/alvgaona/glance/commit/cec7e3c2ba0d2ae38e3e551b089f14599a30a520))
- share list scrolling and details scroll ([4f753de](https://github.com/alvgaona/glance/commit/4f753deb0811da522dd9dd12fed7fed616b4941f))
- split widgets into tui/kit ([444c711](https://github.com/alvgaona/glance/commit/444c7113cd3ff75f432adbe0c09a3fb01a48dc01))
- one event loop for every screen ([c76a453](https://github.com/alvgaona/glance/commit/c76a4538d7c82f067c1f4c2f59b7d2f28a7eee8b))
- move git writes into git/ ([a66b611](https://github.com/alvgaona/glance/commit/a66b611852322bb8ce09444db153fd77d5bad66b))
- reorganize modules by responsibility ([a12e1b4](https://github.com/alvgaona/glance/commit/a12e1b4a861441430e6ae7d596f586d03e532f02))
