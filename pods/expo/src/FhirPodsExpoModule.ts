import { requireNativeModule } from "expo-modules-core";

// Binds to the native module registered by `FhirPodsExpoModule.swift`
// (Name("FhirPodsExpo")).
export default requireNativeModule("FhirPodsExpo");
