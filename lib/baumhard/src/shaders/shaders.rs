pub static SHADERS: [(&'static str, &'static str); 2] = [
    (SHADER_TEST, include_str!("test_shader.wgsl")),
    (SHADER_TEST_TWO, include_str!("test_shader.wgsl")),
];

pub(crate) static SHADER_TEST: &str = "TestShader";
pub(crate) static SHADER_TEST_TWO: &str = "TestShaderTwo";

pub static SHADER_APPLICATION: &str = SHADER_TEST;
