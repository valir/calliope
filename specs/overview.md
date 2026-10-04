# calliope: overview

<!-- The big picture, written by you. Every agent reads this before working on any feature.
     Keep it current: when the vision changes, change this first. Feature specs go in their
     own files next to this one. -->

Calliope is a music player assistant. For now we focus on the guitar players.

Playing music involves some precise timing operation, so we opt for a native
application, able to run on Linux, Windows or Mac. We focus on Linux for now,
more precisely, Arch Linux. Application can use a remote server running ollama
and able to perform AI tasks on the edge. Server is located on the LAN, we
configured it in the ~/edge-ai directory.

The guitar player launches the application named calliope-gui on the Linux
laptop and from there they can perform one of the following operations:
* import an existing music track and perform stem extraction
* import an existing backing track and perform adjustments on it
* manipulate the backing track repository
* import a guitar tablature and associated it with a backing track
* play a backing track while displaying the associated tablature in the app
  window

calliope-gui has a configuration section that enables it:
* to locate the USB-MIDI interface
* to specify where the audio output should go

The MIDI output is used by the application to send the application's beating
clock and patch change commands to the music gear connected on the midi bus.

Guitar tablatures are being handled by external software and Calliope will
only display them. For Linux there is TuxGuitar and for Windows we also have
the GuitarPro program.

Calliope-gui is able to run on Linux/Windows/Mac. It's written using Rust and
it uses Tauri for GUI rendering.

## Vision
<!-- What the whole system does, for whom, and why. One paragraph. -->

Calliope helps musicians improve their music playing skills.

## Use cases
<!-- The main things people do with it, end to end. Short numbered scenarios. -->
1. Manage a backing track repository in a local directory.
2. Import an existing music track and perform stem extraction using the local
   edge-AI powered system. The imported track stems are made available in the
   track repository
3. Recompose the stems in a full backing track with one of the original
   guitars removed
4. Detect the track BPM; some tracks can have multiple sections with different
   BPMs. Add this information to the track metadata
5. Associate a guitar tablature with the backing track; guitar tablatures
   usually are available in GuitarPro format
6. Insert MIDI patch controls in arbitrary points around the backing track
7. Import an existing backing track and perform above operations
8. Navigate to a backing track and start playing it back together with
   assorted MIDI output commands
9. Manage backing track playing lists

## Physical setup
<!-- Every computer and piece of gear involved. -->
| Name         | What it is                              | OS / firmware         | Location / network | Role in the system    |
|--------------|-----------------------------------------|-----------------------|--------------------|-----------------------|
| Laptop       | Musicians tool running the calliope-gui | ArchLinux             | LAN                | Main control point    |
| Server       | ollama-powered server for Edge-AI tasks | ArchLinux             | archserver         | Edge AI tasks         |
| calliope-gui | Calliope main user interface            | ArchLinux/Windows/Mac | laptop             | Main user interaction |


### External gear
<!-- For each device: model, how it connects (USB, serial, MIDI, Ethernet, GPIO…),
     protocol or API, and a link to its manual/spec if you have one. -->

| Name             | What it is                                                      | OS / firmware | Location / network | Role in the system                                         |
|------------------|-----------------------------------------------------------------|---------------|--------------------|------------------------------------------------------------|
| Laptop           | Musicians tool running the calliope-gui                         | ArchLinux     | LAN                | Main control point                                         |
| Server           | ollama-powered server for Edge-AI tasks                         | ArchLinux     | archserver         | Edge AI tasks                                              |
| Roland FC-300    | MIDI controller having MIDI-IN connected to the laptop USB port | Roland        | MIDI               | Emit Patch Changes to the other music gear connected to it |
| Line 6 HX Stomp  | Guitar effects processor and amplifier simulator                | 3.8           | MIDI               | Shape the guitar's tone                                    |
| Digitech Whammy  | Guitar effects processor                                        |               | MIDI               | Perform pitch shifting effects                             |
| Marshall JVM410C | Guitar amplifier                                                |               | MIDI               | Real-world amplifier for live performance                  |

Roland FC-300 manual: ~/nextcloud/chitara/FC-300_OM.pdf
Line 6 manual: "~/nextcloud/chitara/HX Stomp 3.80 Owner's Manual - English .pdf"
Edge-AI setup can be found in the ~/edge-ai directory

## Components and connections
<!-- What software runs where, and how the parts talk to each other. A rough sketch is
     enough; the architect refines it. ASCII diagrams are welcome. -->

## Constraints
<!-- Latency/timing (real-time?), offline operation, power, security, budget,
     languages or tools you want or refuse, things that must never happen. -->

Player audio and tablature view should always stay in sync.
The MIDI commands should be sent in a timely fashion, with the shortest latency
possible.

## Hardware access and testing
<!-- The agents can't touch real gear. Say how features can be tested without it:
     simulators, recorded data, loopback devices. Also list what only you can verify
     on the real hardware. -->

I will use the application for plying backing tracks so we'll have a good
tester for the features.

## Feature roadmap
<!-- The features, in build order, one spec file each. Tick them off as they're built. -->
- [x] `gui-skeleton.md`: calliope-gui skeleton written using Rust and Tauri
- [ ] `gui-tracks-repository.md`: manage the backing track repository
- [ ] `gui-stem-extracting.md`: calliope imports a music track and places stems in the repository
- [ ] `gui-backing-track-assembly.md`: creater backing track out of extracted stems
- [ ] `gui-existing-track-import.md`: import an existing backing track from various sources
- [ ] `gui-tablatures.md`: associate tablature to backing track
- [ ] `gui-manipulate-backing-track.md`: associate tempo and MIDI commands into the backing track
- [ ] `gui-play-backing-track.md`: start playing a backing track

## Glossary
<!-- Domain words and names, so all agents use them consistently. -->

## Open questions

* We are not sure if the edge AI server would need a local dedicated service
  to handle the AI-related tasks such as the stem extraction
* The repository is a locdirectory but the metadata organization might require
  a database; thing is to keep it not so complicated and easy to hack with

