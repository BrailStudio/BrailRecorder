# Tests

`presets_and_capability.rs` and `pipeline_behavior.rs` are pure-logic tests
that run anywhere, including CI on Linux:

```
cargo test -p brail-tests
```

`windows_integration.rs` needs a real Windows machine and is `#[ignore]`d
by default. Run it explicitly:

```powershell
cargo test -p brail-tests --test windows_integration -- --ignored --nocapture
```

Two of those read credentials from the environment and skip themselves
(rather than fail) when unset:

```powershell
$env:BRAIL_TEST_RTMP_URL = "rtmp://a.rtmp.youtube.com/live2"
$env:BRAIL_TEST_STREAM_KEY = "your-key"
$env:BRAIL_LEAK_TEST_MINUTES = "30"
```
