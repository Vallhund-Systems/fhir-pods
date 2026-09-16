Pod::Spec.new do |s|
  s.name           = 'FhirPodsExpo'
  s.version        = '0.3.0'
  s.summary        = 'Embeddable FHIR verifier (expo pod) — Rust core over a lean C ABI.'
  s.license        = { :type => 'MIT' }
  s.author         = 'Justin Grosvenor'
  s.homepage       = 'https://github.com/justinGrosvenor/fhir-pods'
  s.platforms      = { :ios => '15.1' }
  s.source         = { :git => '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  # Vendors the XCFramework produced by scripts/build-ios-xcframework.sh
  # (device + simulator slices of libfhir_pods_ffi.a + the C header/modulemap).
  s.vendored_frameworks = 'Frameworks/FhirPodsFFI.xcframework'

  s.source_files   = '*.swift'
  s.swift_version  = '5.9'
end
