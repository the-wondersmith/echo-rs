# echo-rs

[![crate](https://img.shields.io/crates/v/echo-rs.svg)](https://crates.io/crates/echo-rs)
[![documentation](https://docs.rs/echo-rs/badge.svg)](https://docs.rs/echo-rs)
<!-- [![tests](https://github.com/the-wondersmith/echo-rs/actions/workflows/tests.yml/badge.svg)](https://github.com/the-wondersmith/echo-rs/actions) -->
<!-- [![coverage](https://coveralls.io/repos/github/the-wondersmith/echo-rs/badge.svg?branch=main)](https://coveralls.io/github/the-wondersmith/echo-rs?branch=main) -->

`echo-rs` provides a dead-simple HTTP echo server - i.e. it simply
parrots back any request it receives in a developer-friendly JSON-serialized
format.

It aims to provide a simple and convenient utility to assist in designing
new applications, developing / testing API clients, or as a "dummy" workload
for use in scaffolding new Kubernetes / cloud-native services.

### License

[`AGPL-3.0-or-later`](https://spdx.org/licenses/AGPL-3.0-or-later.html)


### Features
`echo-rs` provides:

- A simple HTTP echo server, returning a JSON-serialized representation of any request made to it
- Prometheus metrics (helpful when using `echo-rs` as a dummy workload when designing new Kubernetes services)


### Basic Usage

Run the server locally -

```bash
docker run -it -p 8080:8080 --rm docker.io/thewondersmith/echo-rs:latest --metrics=false --log-level=debug
```

Then make a request to it with any HTTP client -

```bash
curl -X POST \
  "http://localhost:8080/some/super-cool/endpoint?param1=some-param-value&param2=another-param-value" \
  -H 'Content-Type: application/json' \
  -H 'App-Specific-Header: app_specific_value' \
  -d '{"target": "echo-rs", "expected": "response", "some": ["more", "none", null], "nested": {"turtles": {"all": {"the": {"way": "down"}}}}}'
```

`echo-rs` should return the JSON-serialized representation of the request - 

```json
{
  "method": "POST",
  "path": "/some/super-cool/endpoint",
  "headers": {
    "user-agent": "curl/7.87.0",
    "host": "localhost:8080",
    "accept": "*/*",
    "content-type": "application/json",
    "app-specific-header": "app_specific_value",
    "content-length": "135"
  },
  "params": {
    "param1": "some-param-value",
    "param2": "another-param-value"
  },
  "body": {
    "expected": "response",
    "nested": {
      "turtles": {
        "all": {
          "the": {
            "way": "down"
          }
        }
      }
    },
    "some": [
      "more",
      "none",
      null
    ],
    "target": "echo-rs"
  }
}
```

### TODO:
- Tests 😅