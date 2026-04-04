`proto.rs` can be generated via prost_build tool.

To generate a new version of protobuf file:

```
cd scripts/prost_build
OUT_DIR=your_directory cargo run
```

new protobuf files will be generated in `your_directory` directory.
