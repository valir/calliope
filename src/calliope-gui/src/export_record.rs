//! The export info kept in a track's `track.json` (`exports`): lenient read and strict,
//! JSON-level set / clear. Pure logic, std + serde_json. See plan section 2.2 of
//! `specs/gui-export-info.plan.md`.

use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

/// The top-level key in `track.json`; `track_meta::revision` ignores it.
pub const EXPORTS_KEY: &str = "exports";

/// The key inside `exports` that holds the track's slow count-in switch.
pub const SLOW_COUNT_IN_KEY: &str = "slow_count_in";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rc5Export {
    pub memory: u8,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rc500Export {
    pub memory: u8,
    pub track: u8,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileExport {
    pub path: String,
    pub at: String,
}

/// Every kind is serialized, with `null` for a missing one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TrackExports {
    pub rc5: Option<Rc5Export>,
    pub rc500: Option<Rc500Export>,
    pub mp3: Option<FileExport>,
    pub ogg: Option<FileExport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportEntry {
    Rc5 { memory: u8 },
    Rc500 { memory: u8, track: u8 },
    Mp3 { path: String },
    Ogg { path: String },
}

fn at_of(o: &Map<String, Value>) -> String {
    o.get("at")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn file_entry(o: Option<&Value>) -> Option<FileExport> {
    let o = o?.as_object()?;
    let path = o.get("path")?.as_str()?;
    if path.is_empty() || !Path::new(path).is_absolute() {
        return None;
    }
    Some(FileExport {
        path: path.to_string(),
        at: at_of(o),
    })
}

/// Reads the `exports` value leniently: an entry that is wrong in any way is just absent.
pub fn read(v: Option<&Value>) -> TrackExports {
    let Some(obj) = v.and_then(Value::as_object) else {
        return TrackExports::default();
    };
    let rc5 = obj.get("rc5").and_then(Value::as_object).and_then(|o| {
        let memory = o.get("memory")?.as_u64().filter(|n| (1..=99).contains(n))? as u8;
        Some(Rc5Export {
            memory,
            at: at_of(o),
        })
    });
    let rc500 = obj.get("rc500").and_then(Value::as_object).and_then(|o| {
        let memory = o.get("memory")?.as_u64().filter(|n| (1..=99).contains(n))? as u8;
        let track = o.get("track")?.as_u64().filter(|n| (1..=2).contains(n))? as u8;
        Some(Rc500Export {
            memory,
            track,
            at: at_of(o),
        })
    });
    TrackExports {
        rc5,
        rc500,
        mp3: file_entry(obj.get("mp3")),
        ogg: file_entry(obj.get("ogg")),
    }
}

/// The recorded RC-5 memory of a raw `track.json` object.
pub fn rc5_memory(obj: &Map<String, Value>) -> Option<u8> {
    read(obj.get(EXPORTS_KEY)).rc5.map(|e| e.memory)
}

/// The recorded RC-500 `(memory, track)` of a raw `track.json` object.
pub fn rc500_slot(obj: &Map<String, Value>) -> Option<(u8, u8)> {
    read(obj.get(EXPORTS_KEY))
        .rc500
        .map(|e| (e.memory, e.track))
}

/// Writes only `exports.<kind>`; other keys inside `exports` are kept, a non-object `exports`
/// is replaced.
pub fn set(obj: &mut Map<String, Value>, entry: &ExportEntry, at: &str) {
    let (kind, mut value) = match entry {
        ExportEntry::Rc5 { memory } => (
            "rc5",
            Map::from_iter([("memory".to_string(), Value::from(*memory))]),
        ),
        ExportEntry::Rc500 { memory, track } => (
            "rc500",
            Map::from_iter([
                ("memory".to_string(), Value::from(*memory)),
                ("track".to_string(), Value::from(*track)),
            ]),
        ),
        ExportEntry::Mp3 { path } => (
            "mp3",
            Map::from_iter([("path".to_string(), Value::from(path.as_str()))]),
        ),
        ExportEntry::Ogg { path } => (
            "ogg",
            Map::from_iter([("path".to_string(), Value::from(path.as_str()))]),
        ),
    };
    value.insert("at".into(), Value::from(at));
    let slot = obj
        .entry(EXPORTS_KEY)
        .or_insert_with(|| Value::Object(Map::new()));
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    slot.as_object_mut()
        .expect("just made an object")
        .insert(kind.into(), Value::Object(value));
}

/// The slow count-in switch of an `exports` value: true only for the JSON value `true`.
pub fn slow_count_in(exports: Option<&Value>) -> bool {
    exports
        .and_then(Value::as_object)
        .and_then(|o| o.get(SLOW_COUNT_IN_KEY))
        .is_some_and(|v| v == &Value::Bool(true))
}

/// Sets `exports.slow_count_in`. On inserts `true` (making `exports` an object when needed);
/// off removes the key, and `exports` when it ends up empty. True when something changed.
pub fn set_slow_count_in(obj: &mut Map<String, Value>, on: bool) -> bool {
    if on {
        let slot = obj
            .entry(EXPORTS_KEY)
            .or_insert_with(|| Value::Object(Map::new()));
        if !slot.is_object() {
            *slot = Value::Object(Map::new());
        }
        let o = slot.as_object_mut().expect("just made an object");
        if o.get(SLOW_COUNT_IN_KEY) == Some(&Value::Bool(true)) {
            return false;
        }
        o.insert(SLOW_COUNT_IN_KEY.into(), Value::Bool(true));
        true
    } else {
        let Some(exports) = obj.get_mut(EXPORTS_KEY).and_then(Value::as_object_mut) else {
            return false;
        };
        if exports.remove(SLOW_COUNT_IN_KEY).is_none() {
            return false;
        }
        if exports.is_empty() {
            obj.remove(EXPORTS_KEY);
        }
        true
    }
}

/// Removes `exports.rc5` (and `exports` when it ends up empty). True when something changed.
pub fn clear_rc5(obj: &mut Map<String, Value>) -> bool {
    clear_kind(obj, "rc5")
}

/// Removes `exports.rc500` (and `exports` when it ends up empty). True when something changed.
pub fn clear_rc500(obj: &mut Map<String, Value>) -> bool {
    clear_kind(obj, "rc500")
}

fn clear_kind(obj: &mut Map<String, Value>, key: &str) -> bool {
    let Some(exports) = obj.get_mut(EXPORTS_KEY).and_then(Value::as_object_mut) else {
        return false;
    };
    if exports.remove(key).is_none() {
        return false;
    }
    if exports.is_empty() {
        obj.remove(EXPORTS_KEY);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn reads_valid_entries() {
        let v = json!({
            "rc5": {"memory": 7, "at": "2026-10-24T18:30:00Z"},
            "mp3": {"path": "/m/a.mp3", "at": "2026-10-24T18:32:05Z"},
            "ogg": {"path": "/m/a.ogg", "at": "t"}
        });
        let e = read(Some(&v));
        assert_eq!(
            e.rc5,
            Some(Rc5Export {
                memory: 7,
                at: "2026-10-24T18:30:00Z".into()
            })
        );
        assert_eq!(e.mp3.unwrap().path, "/m/a.mp3");
        assert_eq!(e.ogg.unwrap().at, "t");
        assert_eq!(rc5_memory(&obj(json!({"exports": v}))), Some(7));
    }

    #[test]
    fn read_is_lenient() {
        for bad in [
            json!(0),
            json!(100),
            json!("7"),
            json!(7.5),
            json!(-1),
            json!(null),
        ] {
            assert_eq!(
                read(Some(&json!({"rc5": {"memory": bad}}))).rc5,
                None,
                "{bad}"
            );
        }
        assert_eq!(
            read(Some(&json!({"rc5": {"memory": 99}})))
                .rc5
                .unwrap()
                .memory,
            99
        );
        let e = read(Some(&json!({
            "mp3": {"path": "rel/a.mp3", "at": "t"},
            "ogg": {"path": "", "at": "t"}
        })));
        assert_eq!((e.mp3, e.ogg), (None, None));
        // a missing or non-string `at` is empty
        let e = read(Some(
            &json!({"rc5": {"memory": 3}, "mp3": {"path": "/a.mp3", "at": 5}}),
        ));
        assert_eq!(e.rc5.unwrap().at, "");
        assert_eq!(e.mp3.unwrap().at, "");
        for v in [
            json!("x"),
            json!([1]),
            json!(null),
            json!({"rc5": 3, "mp3": "x"}),
        ] {
            assert_eq!(read(Some(&v)), TrackExports::default(), "{v}");
        }
        assert_eq!(read(None), TrackExports::default());
    }

    #[test]
    fn set_replaces_only_its_kind_and_keeps_unknown_ones() {
        let mut o = obj(json!({"title": "T", "exports": {
            "rc5": {"memory": 1, "at": "old"},
            "mp3": {"path": "/a.mp3", "at": "old"},
            "flac": {"path": "/a.flac"}
        }}));
        set(&mut o, &ExportEntry::Rc5 { memory: 9 }, "new");
        assert_eq!(o["exports"]["rc5"], json!({"memory": 9, "at": "new"}));
        assert_eq!(o["exports"]["mp3"], json!({"path": "/a.mp3", "at": "old"}));
        assert_eq!(o["exports"]["flac"], json!({"path": "/a.flac"}));
        set(
            &mut o,
            &ExportEntry::Ogg {
                path: "/b.ogg".into(),
            },
            "n2",
        );
        assert_eq!(o["exports"]["ogg"], json!({"path": "/b.ogg", "at": "n2"}));
        assert_eq!(o["title"], "T");
        for bad in [json!("x"), json!(null), json!([1])] {
            let mut o = obj(json!({"exports": bad}));
            set(
                &mut o,
                &ExportEntry::Mp3 {
                    path: "/c.mp3".into(),
                },
                "t",
            );
            assert_eq!(o["exports"], json!({"mp3": {"path": "/c.mp3", "at": "t"}}));
        }
        let mut o = Map::new();
        set(&mut o, &ExportEntry::Rc5 { memory: 2 }, "t");
        assert_eq!(o["exports"], json!({"rc5": {"memory": 2, "at": "t"}}));
    }

    #[test]
    fn clear_rc5_removes_exports_when_empty() {
        let mut o = obj(json!({"exports": {"rc5": {"memory": 1, "at": "t"}}}));
        assert!(clear_rc5(&mut o));
        assert!(!o.contains_key("exports"));
        assert!(!clear_rc5(&mut o));

        let mut o = obj(json!({"exports": {"rc5": {"memory": 1}, "mp3": {"path": "/a.mp3"}}}));
        assert!(clear_rc5(&mut o));
        assert_eq!(o["exports"], json!({"mp3": {"path": "/a.mp3"}}));
        assert!(!clear_rc5(&mut o), "no rc5 entry: nothing changes");

        let mut o = obj(json!({"exports": "junk"}));
        assert!(!clear_rc5(&mut o));
        assert_eq!(o["exports"], "junk");
    }

    #[test]
    fn reads_rc500_leniently() {
        let v = json!({"rc500": {"memory": 7, "track": 2, "at": "t"}});
        assert_eq!(
            read(Some(&v)).rc500,
            Some(Rc500Export {
                memory: 7,
                track: 2,
                at: "t".into()
            })
        );
        assert_eq!(rc500_slot(&obj(json!({"exports": v}))), Some((7, 2)));
        for bad in [json!(0), json!(3), json!("2"), json!(1.5), json!(null)] {
            let v = json!({"rc500": {"memory": 7, "track": bad}});
            assert_eq!(read(Some(&v)).rc500, None, "{bad}");
        }
        for v in [
            json!({"rc500": {"memory": 7}}),
            json!({"rc500": {"memory": 0, "track": 1}}),
            json!({"rc500": {"memory": 100, "track": 1}}),
            json!({"rc500": 5}),
        ] {
            assert_eq!(read(Some(&v)).rc500, None, "{v}");
        }
    }

    #[test]
    fn set_rc500_keeps_the_other_kinds() {
        let mut o = obj(json!({"exports": {
            "rc5": {"memory": 1, "at": "old"},
            "mp3": {"path": "/a.mp3", "at": "old"},
            "flac": 5
        }}));
        set(
            &mut o,
            &ExportEntry::Rc500 {
                memory: 7,
                track: 2,
            },
            "new",
        );
        assert_eq!(
            o["exports"]["rc500"],
            json!({"memory": 7, "track": 2, "at": "new"})
        );
        assert_eq!(o["exports"]["rc5"], json!({"memory": 1, "at": "old"}));
        assert_eq!(o["exports"]["mp3"], json!({"path": "/a.mp3", "at": "old"}));
        assert_eq!(o["exports"]["flac"], 5);
    }

    #[test]
    fn clear_rc500_keeps_rc5_and_removes_an_empty_exports() {
        let mut o = obj(json!({"exports": {"rc500": {"memory": 1, "track": 1}}}));
        assert!(clear_rc500(&mut o));
        assert!(!o.contains_key("exports"));
        assert!(!clear_rc500(&mut o));

        let mut o =
            obj(json!({"exports": {"rc5": {"memory": 1}, "rc500": {"memory": 1, "track": 2}}}));
        assert!(clear_rc500(&mut o));
        assert_eq!(o["exports"], json!({"rc5": {"memory": 1}}));
        assert!(!clear_rc500(&mut o), "nothing to remove");

        let mut o = obj(json!({"exports": "junk"}));
        assert!(!clear_rc500(&mut o));
    }

    #[test]
    fn reads_the_slow_count_in_switch() {
        assert!(slow_count_in(Some(&json!({"slow_count_in": true}))));
        for v in [
            json!({"slow_count_in": false}),
            json!({"slow_count_in": "yes"}),
            json!({"slow_count_in": 1}),
            json!({"slow_count_in": null}),
            json!({}),
            json!("x"),
            json!(true),
        ] {
            assert!(!slow_count_in(Some(&v)), "{v}");
        }
        assert!(!slow_count_in(None));
        let with = json!({"rc5": {"memory": 3, "at": "t"}, "slow_count_in": true});
        let without = json!({"rc5": {"memory": 3, "at": "t"}});
        assert_eq!(read(Some(&with)), read(Some(&without)));
    }

    #[test]
    fn sets_the_slow_count_in_switch() {
        let mut o = Map::new();
        assert!(set_slow_count_in(&mut o, true));
        assert_eq!(o["exports"], json!({"slow_count_in": true}));
        assert!(!set_slow_count_in(&mut o, true), "already on");
        assert!(set_slow_count_in(&mut o, false));
        assert!(!o.contains_key("exports"), "emptied exports is removed");
        assert!(!set_slow_count_in(&mut o, false), "nothing to remove");

        for bad in [json!("x"), json!(null), json!([1])] {
            let mut o = obj(json!({"exports": bad}));
            assert!(set_slow_count_in(&mut o, true));
            assert_eq!(o["exports"], json!({"slow_count_in": true}));
        }
        let mut o = obj(json!({"exports": "junk"}));
        assert!(!set_slow_count_in(&mut o, false));
        assert_eq!(o["exports"], "junk");

        let mut o = obj(json!({"exports": {
            "rc5": {"memory": 1, "at": "t"}, "mp3": {"path": "/a.mp3"}, "flac": 5, "slow_count_in": false
        }}));
        assert!(set_slow_count_in(&mut o, true));
        assert_eq!(o["exports"]["slow_count_in"], true);
        assert_eq!(o["exports"]["flac"], 5);
        assert_eq!(o["exports"]["mp3"], json!({"path": "/a.mp3"}));
        assert!(set_slow_count_in(&mut o, false));
        assert_eq!(
            o["exports"],
            json!({"rc5": {"memory": 1, "at": "t"}, "mp3": {"path": "/a.mp3"}, "flac": 5})
        );
    }

    #[test]
    fn serializes_missing_kinds_as_null() {
        let e = TrackExports {
            mp3: Some(FileExport {
                path: "/a".into(),
                at: "t".into(),
            }),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"rc5": null, "rc500": null, "mp3": {"path": "/a", "at": "t"}, "ogg": null})
        );
    }
}
