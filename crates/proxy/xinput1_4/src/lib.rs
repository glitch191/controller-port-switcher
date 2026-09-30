//! xinput1_4.dll: exports the same names as the system DLL (ordinals are set in exports.def).
#![no_std]

xinput_proxy::export!(DllMain XInputGetState XInputSetState XInputGetCapabilities XInputEnable XInputGetBatteryInformation XInputGetKeystroke XInputGetAudioDeviceIds XInputGetStateEx XInputWaitForGuideButton XInputCancelGuideButtonWait XInputPowerOffController XInputGetBaseBusInformation XInputGetCapabilitiesEx XInputOrdinal109 );
