using System.Text.Json;
using Hl7.Fhir.Model;
using Hl7.Fhir.Serialization;
using Hl7.Fhir.Specification.Source;
using Hl7.Fhir.Specification.Terminology;
using Hl7.Fhir.Validation;

if (args.Length != 1) throw new ArgumentException("Usage: sdk-oracle PACKAGE_DIRECTORIES_JSON");
var directories = JsonSerializer.Deserialize<string[]>(File.ReadAllText(args[0]))!;
var sources = directories.Select(p => (IResourceResolver)new DirectorySource(p, new DirectorySourceSettings { IncludeSubDirectories = false })).ToArray();
var resolver = new CachedResolver(new MultiResolver(sources));
var validator = new Validator(new ValidationSettings {
    ResourceResolver = resolver,
    TerminologyService = new LocalTerminologyService(resolver),
    GenerateSnapshot = true,
    ResolveExternalReferences = false
});
var parser = new FhirJsonParser(new ParserSettings { AcceptUnknownMembers = false, AllowUnrecognizedEnums = false });
var serializer = new FhirJsonSerializer();
for (string? line; (line = Console.ReadLine()) != null;) {
    using var request = JsonDocument.Parse(line);
    string id = request.RootElement.GetProperty("id").GetString()!;
    string path = request.RootElement.GetProperty("path").GetString()!;
    Resource resource;
    string raw = File.ReadAllText(path);
    try { resource = parser.Parse<Resource>(raw); }
    catch (FormatException e) {
        Console.WriteLine(JsonSerializer.Serialize(new { id, parse_error = e.Message }));
        continue;
    }
    try {
        using var original = JsonDocument.Parse(raw);
        using var converted = JsonDocument.Parse(serializer.SerializeToString(resource));
        var shapeChanges = new List<string>();
        CompareShape(original.RootElement, converted.RootElement, "$", shapeChanges);
        var outcome = validator.Validate(resource);
        using var json = JsonDocument.Parse(serializer.SerializeToString(outcome));
        Console.WriteLine(JsonSerializer.Serialize(new { id, outcome = json.RootElement, input_shape_changes = shapeChanges }));
    } catch (Exception e) {
        Console.WriteLine(JsonSerializer.Serialize(new { id, execution_error = e.GetType().Name + ": " + e.Message }));
    }
}

// The POCO adapter can coerce JSON wire shapes. Report those cases as
// unscored rather than attributing a verdict on the converted object to the
// original bytes. This checks structure/types, not textual serialisation.
static void CompareShape(JsonElement before, JsonElement after, string path, List<string> changes) {
    if (before.ValueKind != after.ValueKind) {
        changes.Add(path + ": " + before.ValueKind + " -> " + after.ValueKind);
        return;
    }
    if (before.ValueKind == JsonValueKind.Object) {
        foreach (var member in before.EnumerateObject()) {
            if (after.TryGetProperty(member.Name, out var value)) CompareShape(member.Value, value, path + "." + member.Name, changes);
            else changes.Add(path + "." + member.Name + ": removed by parser/model round trip");
        }
    } else if (before.ValueKind == JsonValueKind.Array) {
        if (before.GetArrayLength() != after.GetArrayLength()) changes.Add(path + ": array length changed");
        for (int i = 0; i < Math.Min(before.GetArrayLength(), after.GetArrayLength()); i++) CompareShape(before[i], after[i], path + "[" + i + "]", changes);
    }
}
