require 'json'

# One version: the package's (pods/expo/package.json), bumped by the release.
package = JSON.parse(File.read(File.join(__dir__, '..', 'package.json')))

Pod::Spec.new do |s|
  s.name           = 'FhirPodsExpo'
  s.version        = package['version']
  s.summary        = 'Embeddable FHIR compliance runtime (expo pod) — Rust core over a lean C ABI.'
  s.license        = { :type => 'MIT' }
  s.author         = 'Justin Grosvenor'
  s.homepage       = 'https://github.com/Vallhund-Systems/fhir-pods'
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
