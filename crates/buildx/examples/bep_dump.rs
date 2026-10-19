//! `cargo run -p buildx --example bep_dump -- [--events | --kind <kind>] file.bep`:
//! the event kinds in a binary BEP file, whether its graph is consistent, and the
//! build's exit code. `--events` lists ids and children, `--kind` prints the
//! payloads of one kind, `--cmdline` the options of the command line events and
//! `--metrics` the fields of `build_metrics` that say how the build ran.

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .expect("usage: bep_dump [--events | --kind <kind>] <file.bep>");
    let verbose = first == "--events";
    let payload_kind = if first == "--kind" { args.next() } else { None };
    let compact = first == "--cmdline" || first == "--metrics";
    let path = if verbose || payload_kind.is_some() || compact {
        args.next().expect("a file")
    } else {
        first.clone()
    };
    let bytes = std::fs::read(&path).expect("read");
    let events = buildx::bep::read_events(&bytes).expect("decode");
    if first == "--cmdline" {
        cmdline(&events);
        return;
    }
    if first == "--metrics" {
        metrics(&events);
        return;
    }
    if let Some(k) = &payload_kind {
        for e in events.iter().filter(|e| buildx::bep::kind(e) == k) {
            println!("{:#?}", e.payload);
        }
        return;
    }
    if verbose {
        for e in &events {
            println!("{:?}\n  children: {:?}", e.id, e.children);
        }
    }
    for (kind, n) in buildx::bep::histogram(&events) {
        println!("{n:6} {kind}");
    }
    println!("events: {}", events.len());
    println!("graph (ordered): {:?}", buildx::bep::check_graph(&events));
    println!(
        "graph (complete): {:?}",
        buildx::bep::check_complete(&events)
    );
    println!("exit: {:?}", buildx::bep::exit_code(&events));
}

/// The options of each command line event, one per line, without the client environment.
fn cmdline(events: &[axl_proto::build_event_stream::BuildEvent]) {
    use axl_proto::build_event_stream::build_event::Payload;
    use axl_proto::command_line::command_line_section::SectionType;
    for e in events {
        match &e.payload {
            Some(Payload::UnstructuredCommandLine(c)) => {
                println!("unstructured_command_line:");
                for a in c.args.iter().filter(|a| !a.starts_with("--client_env=")) {
                    println!("  {a}");
                }
            }
            Some(Payload::StructuredCommandLine(c)) => {
                println!("structured_command_line {}:", c.command_line_label);
                for section in &c.sections {
                    match &section.section_type {
                        Some(SectionType::ChunkList(l)) => {
                            println!("  [{}] {:?}", section.section_label, l.chunk)
                        }
                        Some(SectionType::OptionList(l)) => {
                            for o in l.option.iter().filter(|o| o.option_name != "client_env") {
                                println!(
                                    "  [{}] {} name={:?} value={:?} effects={:?} metadata={:?}",
                                    section.section_label,
                                    o.combined_form,
                                    o.option_name,
                                    o.option_value,
                                    o.effect_tags,
                                    o.metadata_tags
                                );
                            }
                        }
                        None => {}
                    }
                }
            }
            Some(Payload::OptionsParsed(o)) => {
                println!("options_parsed:");
                for a in &o.cmd_line {
                    println!("  cmd_line {a}");
                }
                for a in &o.explicit_cmd_line {
                    println!("  explicit_cmd_line {a}");
                }
            }
            _ => {}
        }
    }
}

fn metrics(events: &[axl_proto::build_event_stream::BuildEvent]) {
    use axl_proto::build_event_stream::build_event::Payload;
    for e in events {
        let Some(Payload::BuildMetrics(m)) = &e.payload else {
            continue;
        };
        if let Some(a) = &m.action_summary {
            println!(
                "actions_created={} actions_created_not_including_aspects={} actions_executed={}",
                a.actions_created, a.actions_created_not_including_aspects, a.actions_executed
            );
            for r in &a.runner_count {
                println!(
                    "runner_count {:?} count={} exec_kind={:?}",
                    r.name, r.count, r.exec_kind
                );
            }
            for d in &a.action_data {
                println!(
                    "action_data {} executed={} created={}",
                    d.mnemonic, d.actions_executed, d.actions_created
                );
            }
            if let Some(c) = &a.action_cache_statistics {
                println!(
                    "action_cache_statistics hits={} misses={} miss_details={:?}",
                    c.hits,
                    c.misses,
                    c.miss_details
                        .iter()
                        .map(|d| (d.reason, d.count))
                        .collect::<Vec<_>>()
                );
            }
        }
        if let Some(t) = &m.target_metrics {
            println!(
                "targets_loaded={} targets_configured={}",
                t.targets_loaded, t.targets_configured
            );
        }
        if let Some(p) = &m.package_metrics {
            println!("packages_loaded={}", p.packages_loaded);
        }
        if let Some(t) = &m.timing_metrics {
            println!(
                "cpu_time_in_ms={} wall_time_in_ms={} analysis_phase_time_in_ms={} execution_phase_time_in_ms={}",
                t.cpu_time_in_ms,
                t.wall_time_in_ms,
                t.analysis_phase_time_in_ms,
                t.execution_phase_time_in_ms
            );
        }
    }
}
