# Tetanux

## What the hell is this?

Tetanux is a small web (HTTP/SSL) proxy server, basically it is a trimmed down version of [Tinyproxy](https://github.com/tinyproxy/tinyproxy) with minimal Tinyproxy's features and a few additional features that added for my use case.

## Why it's named Tetanux?
It was named after word [Tetanus](https://en.wikipedia.org/wiki/Tetanus) that often associated with Rust (as the programming language) and X from the word proxy.

## How to run it?

Of course, it needs the Rust [toolchain](https://rust-lang.org/tools/install/). Since it follow Tinyproxy CLI and configuration format, it can be run just like Tinyproxy.

```
$ git clone git@github.com:ajhwb/tetanux.git
$ cd tetanux
$ cargo build -r
$ ./target/release/tetanux -h
A small web proxy server

Usage: tetanux [OPTIONS]

Options:
  -c <C>         Configuration file
  -h, --help     Print help
  -V, --version  Print version
```

## Configuration

Currently supported Tinyproxy configurations:

* Port
* Listen
* Timeout
* Allow
* Deny
* BasicAuth
* BasicAuthRealm

Please see Tinyproxy [documentation](https://tinyproxy.github.io#documentation) for more details. 

**More configurations will be supported in the future releases**

Additional configurations:

<table>
<th>Name</th>
<th>Description</th>
<th>Values</th>
<tr>
<td>DnsOver</td>
<td>Whether to use DNS over HTTPS/TLS instead of standard DNS</td>
<td>Any of: cloudfare-https, cloudfare-tls, google-https, google-tls, quad9-https, quad9-tls</td>
</tr>
</table>

Example configuration file:

```
Port 8888
Listen 127.0.0.1
Timeout 600
Allow 127.0.0.1
DnsOver quad9-https
```

## License

[GPLv2](https://www.gnu.org/licenses/old-licenses/gpl-2.0.html#SEC1)
