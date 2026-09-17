"""Evidence guards: execution failures and incomplete evidence never become parity."""
import copy
import json
import subprocess
import unittest

from run import local_result, oracle_results, strip_profiles


def outcome(name='example.json'):
    return {'resourceType': 'OperationOutcome',
            'extension': [{'url': 'http://hl7.org/fhir/StructureDefinition/operationoutcome-file', 'valueString': name}],
            'issue': [{'severity': 'information', 'code': 'informational'}]}


def bundle(*outcomes):
    return {'resourceType': 'Bundle', 'entry': [{'resource': o} for o in outcomes]}


class EvidenceTests(unittest.TestCase):
    def test_process_failure_and_verdict_contradictions_rejected(self):
        for code, obj in [(-11, {'valid': True, 'issues': []}), (2, {'valid': True, 'issues': []}),
                          (1, {'valid': True, 'issues': []}), (0, {'valid': False, 'issues': []}),
                          (0, {'valid': True, 'issues': [{'severity': 'error'}]}),
                          (0, {'valid': True, 'issues': [{'severity': 'banana'}]})]:
            with self.subTest(code=code, obj=obj), self.assertRaises(ValueError):
                local_result(subprocess.CompletedProcess([], code, json.dumps(obj), ''))

    def test_genuine_valid_and_invalid_reports_accepted(self):
        for code, obj in [(0, {'valid': True, 'issues': []}),
                          (1, {'valid': False, 'issues': [{'severity': 'error'}]})]:
            self.assertEqual(local_result(subprocess.CompletedProcess([], code, json.dumps(obj), '')), obj)

    def test_missing_duplicate_and_foreign_outcomes_rejected(self):
        for value in [bundle(), bundle(outcome(), outcome()), bundle(outcome('foreign.json')),
                      {'resourceType': 'OperationOutcome', 'issue': []}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                oracle_results(value, ['example.json'])

    def test_malformed_outcomes_rejected(self):
        values = [[], [{'code': 'invalid'}], [{'severity': 'banana', 'code': 'invalid'}]]
        for issues in values:
            value = outcome()
            value['issue'] = issues
            with self.subTest(issues=issues), self.assertRaises(ValueError):
                oracle_results(bundle(value), ['example.json'])
        value = outcome()
        value['extension'].append(copy.deepcopy(value['extension'][0]))
        with self.assertRaises(ValueError):
            oracle_results(bundle(value), ['example.json'])

    def test_exact_outcome_identity_accepted(self):
        value = outcome()
        self.assertEqual(oracle_results(bundle(value), ['example.json']), {'example.json': value})

    def test_recursive_profile_removal_preserves_other_metadata_and_input(self):
        source = {'resourceType': 'Bundle', 'meta': {'profile': ['x'], 'tag': [{'code': 'synthetic'}]},
                  'entry': [{'resource': {'resourceType': 'Patient', 'meta': {'profile': ['y']}}}],
                  'identifier': {'type': {'meta': {'profile': ['not-a-resource']}}}}
        value = copy.deepcopy(source)
        removed = strip_profiles(value)
        self.assertEqual(len(removed), 2)
        self.assertEqual(value['meta'], {'tag': [{'code': 'synthetic'}]})
        self.assertNotIn('meta', value['entry'][0]['resource'])
        self.assertEqual(value['identifier'], source['identifier'])
        self.assertIn('profile', source['meta'])


if __name__ == '__main__':
    unittest.main()
