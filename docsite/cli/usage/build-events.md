

The `ctx.bazel.build` function accepts an `events` parameter. When set to `True`, build events become available through an iterator interface. Build events are delivered synchronously via an iterator, as Starlark operates in a single-threaded environment. Each event contains a `kind` field that corresponds to the event types defined in Bazel's `build_event_stream.proto` protocol specification.

<Info>
Build Event Stream Protocol
Build events follow Bazel's Build Event Stream (BES) protocol. For detailed specifications:
- **Protocol definition**: [build_event_stream.proto](https://github.com/bazelbuild/bazel/blob/master/src/main/java/com/google/devtools/build/lib/buildeventstream/proto/build_event_stream.proto)
- **API documentation**: [Buf Schema Registry](https://buf.build/bazel/bazel/docs/main:build_event_stream)
- **Event ordering examples**: [Bazel BEP Documentation](https://bazel.build/remote/bep-examples)
</Info>


Events also have a `payload` field, whose type depends on which `kind` of event is produced.
See [build_event kinds](/axl/bazel/build/build_event).

For example:
```python
    build = ctx.bazel.build(
        events = True,
        ...
    )
    for event in build.events():
        # conditional logic on event.kind
        if event.kind == 'named_set_of_files':
            for file in event.payload.files:
                pass
        elif event.kind == 'target_complete':
            pass
        else
            pass
```
