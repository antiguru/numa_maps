numa_maps
=======

A simple library to read the `numa_maps` file on Linux.

```toml
[dependencies]
numa_maps = "0.1"
```

## Example

```rust
#[cfg(target_os = "linux")]
let map = numa_maps::NumaMap::from_file("/proc/self/numa_maps").unwrap();
#[cfg(not(target_os = "linux"))]
let map = numa_maps::NumaMap::default();

for region in &map.ranges {
    println!("base: {:x} -> {:?}", region.address, region.properties);
}

```

## Description

`numa_maps` provides a simple API to read the contents of the `numa_maps` file.
It parses the file into ranges with a base address, a policy, and a list of
properties.

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
