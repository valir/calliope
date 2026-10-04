# calliope-gui skeleton

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Produce the calliope-gui binary

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

Application is written using Rust and has Tauri GUI to be OS independent.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. binary created using Rust
2. source code placed in the `src` subdirectory
3. GUI written using Tauri
4. application binary can take arguments
5. applicaiton launched with no arguments show the main GUI
6. application offers the arguments `--help` and `--version`
7. version strings are composed of the date and a sequence number maintained
   at build time, like this: `YY.MM.BBBB` where BBB is the build number
8. when invoked using --help applications shows this output:

```
   calliope-gui, version YY.MM.BBBB
   (c) 2006 Valentin Rusu

   Usage: calliope-gui [options]

   Options:
      --help: produces this output
      --version: produces short string containing the version number
```
9. when invoked using --version applications shows this output:
```
calliope-gui YY.MM.BBB
```


## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

Application is written using Rust and has Tauri GUI to be OS independent.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given source code, when we compile it, then it produces calliope-gui binary file
- [ ] Given binary compiled, when we run it with no arguments, then GUI is visible and displays the "hello from calliope"
- [ ] Given binary compiled, when invoked with `--help`, then it shows the output specified at the requirement #8
- [ ] Given binary compiled, when invoked with `--version`, then it shows the output specified at the requirement #9

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->
