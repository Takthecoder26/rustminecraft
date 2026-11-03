// A standalone portable default; the provider specializes this to the actual device limit.
override LIMIT: u32 = 65535u;
@group(0) @binding(0) var<storage, read_write> args: array<u32, 3>;
@compute @workgroup_size(1) fn main() {
    if args[0] > LIMIT || args[1] > LIMIT || args[2] > LIMIT {
        args[0] = 0u;
        args[1] = 0u;
        args[2] = 0u;
    }
}
