//! xinput9_1_0.dll: exports the same names as the system DLL (ordinals are set in exports.def).
#![no_std]

xinput_proxy::export!(DllMain XInputGetCapabilities XInputGetDSoundAudioDeviceGuids XInputGetState XInputSetState );
