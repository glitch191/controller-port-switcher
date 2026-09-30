//! xinput1_3.dll: exports the same names as the system DLL (ordinals are set in exports.def).
#![no_std]

xinput_proxy::export!(DllMain XInputGetState XInputSetState XInputGetCapabilities XInputEnable XInputGetDSoundAudioDeviceGuids XInputGetBatteryInformation XInputGetKeystroke XInputGetStateEx XInputWaitForGuideButton XInputCancelGuideButtonWait XInputPowerOffController );
