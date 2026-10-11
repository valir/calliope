base: aa9a84b
task 1 | Player test fixtures | implementer | done
task 2 | Reserve the file name player.json | local | done
task 3 | Tempo map | implementer | done
task 4 | Player state and player.json, export | implementer | done
task 5 | Sources, decoding and peaks | implementer | done (plan fixes: raw-stdout spawn in calliope-lib; fixture envelope amplitude)
task 6 | Metronome click and scheduling | implementer | done
task 7 | Player engine core | implementer | done
task 8 | Engine: metronome, count-in and loops | implementer | done
task 9 | IPC commands, shared backend, ticker, exit | implementer | done
task 10 | Bar and BPM formatting | implementer (local model wrote no files) | done
task 11 | Waveform geometry | local (wrote to the wrong folder; moved and verified by the orchestrator) | done
task 12 | Player view layout, collapsible tree, select, settings placeholders | implementer | done
task 13 | Player frontend state | implementer | done
task 14 | Control bar sections | implementer | done
task 15 | Waveform view | implementer | done
task 16 | Player keyboard shortcuts | implementer | done
task 17 | GUI end-to-end tests, visual review, docs | implementer | done
task 18 | Switch the project licence to MPL-2.0 | implementer | done
reviewer | CHANGES REQUIRED: 1 major (player.json note/state-error never shown), minors 2-7, 9; minor 8 disproved (LICENSE byte-identical to mozilla.org) | reviewer | done
tester | 28 criteria PASS; 4 defects (player.json problems not shown; loops unreachable without current_loop; huge tempo start_ms / seek overflow) | tester | done
fix round 1 | player.json problems shown; loops reachable without current_loop; overflow/0-frame guards; key repeat; loop bounds; Loop On look; GUI script | implementer | done
reviewer (fix round 1) | APPROVED; 2 optional minors | reviewer | done
tester (fix round 1) | all 4 defects fixed; new cosmetic: note and state-error overlap in the one-line placeholder box | tester | done
