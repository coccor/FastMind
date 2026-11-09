//! A demo notebook with the mess real notebooks collect: copies of the same note, fragments of
//! one subject scattered about, finished and forgotten notes, an empty note, a mistyped link, a
//! file nothing links to and old archived notes. Dates are set relative to today, so Dream has
//! the same work to do whenever the demo is made.

use crate::archive::{Index, Item};
use crate::notebook::{Notebook, STATE_DIR};
use crate::{Error, Result, fsx};
use chrono::{Duration, Local};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

struct Seed {
    path: &'static str,
    days_ago: i64,
    front: &'static str,
    body: &'static str,
}

const SEEDS: &[Seed] = &[
    // A home note: Dream never touches it.
    Seed {
        path: "work/Work.md",
        days_ago: 3,
        front: "type: project",
        body: "# Work\n\nWhat I'm on and where things are.\n\n- [[Postgres vacuum]]\n- [[Redis eviction policy]]\n- [[Release checklist]]\n- [[Payments service]]\n",
    },
    Seed {
        path: "work/Postgres vacuum.md",
        days_ago: 40,
        front: "tags: [postgres, db]",
        body: "# Postgres vacuum\n\nAutovacuum falls behind on huge, busy tables: dead tuples pile up and queries slow down.\n\nFor the big tables, lower `autovacuum_vacuum_scale_factor` to 0.02 and raise `autovacuum_vacuum_cost_limit` to 2000.\n\nWatch `n_dead_tup` in `pg_stat_user_tables`.\n",
    },
    Seed {
        path: "work/Payments service.md",
        days_ago: 12,
        front: "tags: [payments]",
        body: "# Payments service\n\nOwns charges and refunds. Talks to the ledger through the outbox table.\n\nIncident runbook: [[Release checklist]]. Slow queries: see [[Postgress vacuum]].\n",
    },
    Seed {
        path: "work/Release checklist.md",
        days_ago: 20,
        front: "",
        body: "# Release checklist\n\n1. Bump the version.\n2. Run the full test suite on all three OSes.\n3. Tag, push, watch CI.\n4. Write the release notes.\n",
    },
    // Duplicates: the same note captured twice, once tidied up.
    Seed {
        path: "work/Redis eviction policy.md",
        days_ago: 30,
        front: "tags: [redis]",
        body: "# Redis eviction policy\n\nWhen maxmemory is reached Redis evicts keys according to maxmemory-policy. Use allkeys-lru for a pure cache and volatile-lru when some keys must never be evicted.\n\nSet maxmemory explicitly in production; the default of 0 means no limit on 64-bit systems.\n",
    },
    Seed {
        path: "inbox/Redis eviction.md",
        days_ago: 25,
        front: "type: capture\nstatus: inbox\ntags: [cache]",
        body: "# Redis eviction\n\nWhen maxmemory is reached Redis evicts keys according to maxmemory-policy. Use allkeys-lru for a pure cache and volatile-lru when some keys must never be evicted.\n\nWatch evicted_keys in INFO stats to see whether eviction is happening at all.\n",
    },
    // Three takes on the same Docker trick.
    Seed {
        path: "ideas/Docker build cache.md",
        days_ago: 60,
        front: "tags: [docker]",
        body: "# Docker build cache\n\nCopy the dependency manifests first and install dependencies before copying the source, so a code change doesn't throw away the dependency layer of the build cache.\n",
    },
    Seed {
        path: "inbox/Faster docker builds.md",
        days_ago: 15,
        front: "type: capture\nstatus: inbox",
        body: "# Faster docker builds\n\nCopy the dependency manifests first and install dependencies before copying the source, so a code change doesn't throw away the dependency layer of the build cache.\n\nAlso: BuildKit cache mounts (`--mount=type=cache`) keep the package cache between builds.\n",
    },
    Seed {
        path: "inbox/docker layer caching.md",
        days_ago: 9,
        front: "type: capture\nstatus: inbox",
        body: "# docker layer caching\n\ncopy the dependency manifests first and install dependencies before copying the source so a code change doesn't throw away the dependency layer of the build cache\n",
    },
    // Fragments of one subject, captured on different days.
    Seed {
        path: "inbox/Readiness probe.md",
        days_ago: 18,
        front: "type: capture\nstatus: inbox\ntags: [kubernetes]",
        body: "# Readiness probe\n\nKubernetes readiness probes decide whether a pod gets traffic. A failing readiness probe takes the pod out of the service.\n",
    },
    Seed {
        path: "inbox/Liveness probe restarts.md",
        days_ago: 16,
        front: "type: capture\nstatus: inbox\ntags: [kubernetes]",
        body: "# Liveness probe restarts\n\nA failing liveness probe makes Kubernetes restart the pod. Too strict a probe causes restart loops under load.\n",
    },
    Seed {
        path: "ideas/Startup probe.md",
        days_ago: 14,
        front: "tags: [kubernetes]",
        body: "# Startup probe\n\nSlow-starting pods need a startup probe so the liveness probe doesn't kill them before they're up. Kubernetes holds the other probes until it passes.\n",
    },
    Seed {
        path: "inbox/probe timeouts.md",
        days_ago: 11,
        front: "type: capture\nstatus: inbox",
        body: "# probe timeouts\n\nKubernetes probe timeoutSeconds defaults to 1 second; slow health endpoints fail the probe and the pod restarts.\n",
    },
    // Another cluster, at home.
    Seed {
        path: "home/Sourdough feeding.md",
        days_ago: 50,
        front: "tags: [baking]",
        body: "# Sourdough feeding\n\nFeed the sourdough starter equal weights of flour and water twice a day; it should double within six hours.\n",
    },
    Seed {
        path: "home/Starter too sour.md",
        days_ago: 47,
        front: "tags: [baking]",
        body: "# Starter too sour\n\nA sourdough starter that smells of vinegar is hungry: feed it more often and keep the starter warmer.\n",
    },
    Seed {
        path: "home/Sourdough bake day.md",
        days_ago: 44,
        front: "tags: [baking]",
        body: "# Sourdough bake day\n\nMix the dough when the sourdough starter peaks, then bulk ferment until it grows by half. Bake at 250 °C in a covered pot.\n",
    },
    // Finished and forgotten.
    Seed {
        path: "work/Migrate CI to GitHub Actions.md",
        days_ago: 75,
        front: "status: done",
        body: "# Migrate CI to GitHub Actions\n\n- [x] Port the build job\n- [x] Port the release job\n- [x] Turn off the old CI server\n",
    },
    Seed {
        path: "inbox/Conference wifi.md",
        days_ago: 400,
        front: "type: capture\nstatus: inbox",
        body: "# Conference wifi\n\nNetwork DevConf-Guest, password on the badge.\n",
    },
    Seed {
        path: "ideas/Meeting room booking.md",
        days_ago: 300,
        front: "",
        body: "# Meeting room booking\n\nRoom 4B is booked on Thursdays from 2 to 3 for the design review.\n",
    },
    Seed {
        path: "ideas/Untitled.md",
        days_ago: 90,
        front: "",
        body: "\n",
    },
    // Healthy notes Dream should leave alone.
    Seed {
        path: "ideas/Notebook as memory.md",
        days_ago: 5,
        front: "tags: [fastmind]",
        body: "# Notebook as memory\n\nCapture costs nothing, finding is instant, and the notebook tidies itself at night. See [[Work]].\n",
    },
    Seed {
        path: "home/Reading list.md",
        days_ago: 8,
        front: "pin: true",
        body: "# Reading list\n\n- The Pragmatic Programmer\n- Designing Data-Intensive Applications\n",
    },
];

const ARCHIVED: &[(&str, i64, &str)] = &[
    (
        "work/Q1 planning.md",
        70,
        "# Q1 planning\n\nHire two engineers, ship the payments rewrite, cut cloud spend by a tenth.\n",
    ),
    (
        "ideas/Old blog draft.md",
        65,
        "# Old blog draft\n\nWhy every team should write runbooks before incidents, not during them.\n",
    ),
];

/// Creates the demo notebook at `root`. Refuses a folder that already has notes in it.
pub fn create(root: &Path) -> Result<usize> {
    if root.exists() && fs::read_dir(root)?.next().is_some() {
        if root.join(".fastmind-demo").exists() {
            fs::remove_dir_all(root)?;
        } else {
            return Err(Error::msg(format!(
                "{} already has files in it; choose an empty folder for the demo",
                root.display()
            )));
        }
    }
    let nb = Notebook::open(root)?;
    fs::write(
        nb.root.join(".fastmind-demo"),
        "This folder is FastMind's demo notebook; making the demo again replaces it.\n",
    )?;
    let now = Local::now();
    for seed in SEEDS {
        let when = now - Duration::days(seed.days_ago);
        let date = when.format("%Y-%m-%d").to_string();
        let mut front = format!("created: {date}");
        if !seed.front.is_empty() {
            front = format!("{}\n{front}", seed.front);
        }
        let text = format!("---\n{front}\n---\n{}", seed.body);
        let path = nb.path(seed.path)?;
        fsx::write_atomic(&path, text.as_bytes())?;
        set_mtime(&path, when.into())?;
    }
    // A diagram nothing links to any more.
    let image = nb.path("work/attachments/old architecture.png")?;
    fsx::write_atomic(&image, &fake_png(48 * 1024))?;
    set_mtime(&image, (now - Duration::days(120)).into())?;

    // Notes archived two months ago, ready to be packed.
    let mut index = Index::default();
    for (original, days, body) in ARCHIVED {
        let when = now - Duration::days(*days);
        let stored = format!(
            "{STATE_DIR}/archive/files/{}/{original}",
            when.format("%Y-%m")
        );
        let text = format!(
            "---\ncreated: {}\n---\n{body}",
            (when - Duration::days(30)).format("%Y-%m-%d")
        );
        fsx::write_atomic(&nb.path(&stored)?, text.as_bytes())?;
        index.items.push(Item {
            id: format!("demo-{}", index.items.len() + 1),
            original: (*original).to_owned(),
            stored,
            title: body
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches("# ")
                .to_owned(),
            reason: "archived by hand".into(),
            archived_at: when,
            size: text.len() as u64,
            pack: None,
        });
    }
    for item in &index.items {
        let path = nb.path(&format!("{}/{}.json", crate::archive::items_dir(), item.id))?;
        fsx::write_atomic(&path, &serde_json::to_vec_pretty(item)?)?;
    }
    Ok(SEEDS.len())
}

fn set_mtime(path: &Path, when: SystemTime) -> Result<()> {
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.set_modified(when)?;
    Ok(())
}

/// Bytes that start like a PNG, standing in for a real image.
fn fake_png(size: usize) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut x: u32 = 2463534242;
    while bytes.len() < size {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        bytes.push((x & 0xff) as u8);
    }
    bytes
}
