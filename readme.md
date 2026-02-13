# Sanitarr

Sanitarr is a tool designed to clean up your media library by integrating with
the [**\*arr**](https://wiki.servarr.com/) stack (Radarr, Sonarr), Jellyfin and
multiple download clients of your choice (currently only qBittorrent and Deluge
are supported). It helps you manage and maintain your media collection by
removing fully watched items, thereby reducing the size of your collection on
the disk.

## Features

- Integrates with \*arr stack, Jellyfin and a number of torrent clients;
- Supports multiple concurrently running torrent clients;
- Cleans up movies and series based on your configuration;
- Supports custom tags to keep specific files;
- Provides logging and error handling;

## Configuration

Sanitarr uses a configuration file to specify the settings for each service it
integrates with. Below is an example configuration file in TOML format (all
parameters should be self explanatory). For more details check
[src/config.rs](src/config.rs)

```toml
username = "john"

[jellyfin]
base_url = "http://localhost:8096"
api_key = "sadfa2345234asdfasd2345234"

[radarr]
base_url = "http://localhost:7878"
api_key = "sadfa2345234asdfasd2345234"
tags_to_keep = ["keep"]
retention_period = "2d"

[sonarr]
base_url = "http://localhost:8989"
api_key = "sadfa2345234asdfasd2345234"
tags_to_keep = ["keep", "no_remove"]
retention_period = "1w"

# You can configure multiple download clients running in your system.
# Currently only 'qBittorrent' and 'Deluge' are supported.
# Which client to delete a specific torrent from will be decided
# automatically based on the API response from either Sonarr or Radarr.
# See "History" API reference for more details
# - https://sonarr.tv/docs/api/#v3/tag/history/GET/api/v3/history
# - https://radarr.video/docs/api/#/History/get_api_v3_history

[download_clients.qbittorrent]
base_url = "http://localhost:6880"
username = "admin"
password = "adminadmin"

[download_clients.deluge]
base_url = "http://localhost:8112"
password = "qwerty"
```

## Installation

### From Source

To build and install Sanitarr from source, you need to have [Rust
installed](https://www.rust-lang.org/tools/install). Clone the repository and
run the following commands:

```sh
git clone https://github.com/serzhshakur/sanitarr.git
cd sanitarr
cargo build --release
```

The binary will be located in the `target/release` directory.

### Using Docker

When running Sanitarr in a Docker container, the binary will be executed
periodically at intervals controlled by the `INTERVAL` environment variable. The
value for `INTERVAL` should be specified in a [format understood by the `sleep`
command](https://www.gnu.org/software/coreutils/manual/html_node/sleep-invocation.html#sleep_003a-Delay-for-a-specified-time)
(e.g., `1h` for one hour, `30m` for thirty minutes).

### Pre-built Docker image

Docker images are regularly published to [ghcr.io
registry](https://github.com/serzhshakur/sanitarr/pkgs/container/sanitarr). Note
that currently only amd64 images are published.

### Build locally

You can build and run Sanitarr using Docker:

```sh
docker build -t sanitarr:local .
```

## Running

### Standalone Docker container

```sh
docker run -it \
  --network host \
  -e INTERVAL="1h" \
  -v /path/to/sanitarr-config.toml:/app/config.toml \
  sanitarr:local \
  --log-level debug --config /app/config.toml --force-delete
```

### Docker Compose

You can also use Docker Compose to run Sanitarr. Below is an example
`docker-compose.yml` file:

```yaml
services:
  sanitarr:
    image: ghcr.io/serzhshakur/sanitarr:latest
    container_name: sanitarr
    network_mode: "host"
    pull_policy: never
    environment:
      LOG_LEVEL: debug
      INTERVAL: 45m
    volumes:
      - /path/to/sanitarr-config.toml:/app/config.toml
    command:
      - "--config"
      - "/app/config.toml"
      - "--force-delete"
    depends_on:
      - jellyfin
      - sonarr
      - radarr
```

### Using sanitarr binary executable

To run Sanitarr executable, use the following command:

```sh
sanitarr --config /path/to/config.toml [--log-level] [--force-delete]
```

For more detailed info on CLI arguments consult to `sanitarr --help`:

```
Usage: sanitarr [OPTIONS] --config <CONFIG>

Options:
  -d, --force-delete           Perform actual deletion of files. If not set the program will operate in a "dry run" mode
  -l, --log-level <LOG_LEVEL>  You can either provide a single log level (like `info`) or use a more detailed syntax like `off,sanitarr=debug,reqwest=info` (similar to `tracing_subscriber::filter::EnvFilter` syntax) [env: LOG_LEVEL=]
  -c, --config <CONFIG>        Path to the config file
  -h, --help                   Print help
  -V, --version                Print version
```

You can also specify the log level using the `LOG_LEVEL` environment variable:

```sh
LOG_LEVEL="off,sanitarr=debug" sanitarr
```

## Contributing

Contributions are welcome! Please open an issue or submit a pull request on
GitHub.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
