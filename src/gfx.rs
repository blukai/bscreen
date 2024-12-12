// NOTE: TextureFormat is modeled after webgpu, see:
// - https://github.com/webgpu-native/webgpu-headers/blob/449359147fae26c07efe4fece25013df396287db/webgpu.h
// - https://www.w3.org/TR/webgpu/#texture-formats
pub enum TextureFormat {
    // Bgra8Unorm is compatible with VK_FORMAT_B8G8R8A8_UNORM, it is also
    // compativle with DRM_FORMAT_XRGB8888 (x is not alpha, x means that the byte is
    // wasted).
    Bgra8Unorm,
    Rgba8Unorm,
}
