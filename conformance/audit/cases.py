#!/usr/bin/env python3
"""Fixed synthetic audit conformance examples, including the v0.6 review regressions."""
import base64
import copy


BALP = "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit."

BASE = "http://hl7.org/fhir/StructureDefinition/"

EVENT = {"recorded": "2026-09-16T10:00:00Z", "client": {"who": "Device/synthetic-client", "network": "192.0.2.10"},
         "server": {"who": "Device/synthetic-server", "network": "https://example.org/fhir"},
         "user": {"who": "Practitioner/synthetic-user", "name": "Synthetic Reviewer"},
         "source": {"observer": "Device/synthetic-audit", "site": "Synthetic site"},
         "data": "Observation/synthetic", "transaction": "synthetic-request-id"}

def previous_cases():
    out = []
    def add(id, operation, parameter, inputs, expect, **checks):
        out.append(dict(id=id, operation=operation, parameter=parameter, inputs=copy.deepcopy(inputs), expect=expect, **checks))
    for action, code in [("read", "R"), ("query", "E"), ("create", "C"), ("update", "U"), ("delete", "D")]:
        for patient in (False, True):
            kind = ("patient-" if patient else "") + action
            event = copy.deepcopy(EVENT)
            if patient:
                event["patient"] = "Patient/synthetic"
            if action == "query":
                del event["data"]
                event["query"] = "GET /Observation?patient=synthetic&name=患者 Ω\nsecond\tline"
            for user in (True, False):
                if not user:
                    del event["user"]
                add(kind + ("-user" if user else "-no-user"), "audit", kind, event, "valid",
                    action=code, profile=BALP + ("Patient" if patient else "") + action.title(), agents=3 if user else 2,
                    query_bytes=event.get("query"))
    for path in ["recorded", "client", "client.who", "client.network", "server", "server.who", "server.network", "source", "source.observer", "data"]:
        event = copy.deepcopy(EVENT)
        parts = path.split(".")
        parent = event if len(parts) == 1 else event[parts[0]]
        del parent[parts[-1]]
        add("missing-" + path, "audit", "read", event, "error")
    for path in ["recorded", "client", "client.who", "client.network", "server", "server.who", "server.network", "source", "source.observer", "source.site", "data", "transaction", "user", "user.who", "user.name"]:
        event = copy.deepcopy(EVENT)
        parts = path.split(".")
        parent = event if len(parts) == 1 else event[parts[0]]
        parent[parts[-1]] = 17
        add("malformed-" + path, "audit", "read", event, "error")
    for key, value in [("query", "GET /Patient"), ("patient", "Patient/p"), ("outcome", "4"), ("surprise", True)]:
        add("unexpected-" + key, "audit", "read", {**EVENT, key: value}, "error")
    add("bad-kind", "audit", "browse", EVENT, "error")
    add("not-object", "audit", "read", [], "error")
    for actor in ["client", "server", "user"]:
        event = copy.deepcopy(EVENT)
        event[actor]["requestor"] = False if actor == "user" else True
        add("requestor-" + actor, "audit", "read", event, "invalid" if actor == "user" else "valid")
    for address, expected in [("2001:db8::1", "2"), ("fe80::1%en0", "2"), ("mailto:reviewer@example.org", "5"),
                               ("urn:uuid:00000000-0000-4000-8000-000000000001", "5"), ("synthetic-host", "1"),
                               ("192.0.2.10", "2"), ("https://example.org/fhir", "5"), ("reviewer@example.org", "4")]:
        event = copy.deepcopy(EVENT)
        event["client"]["network"] = address
        add("network-" + str(len(out)), "audit", "read", event, "valid", network_type=expected)
    for who in ["Synthetic Name", "Device/synthetic", "https://example.org/fhir/Device/synthetic", "urn:uuid:00000000-0000-4000-8000-000000000001", "#contained"]:
        event = copy.deepcopy(EVENT)
        event["client"]["who"] = who
        add("who-" + str(len(out)), "audit", "read", event, "valid", observed_who=who)
    event = copy.deepcopy(EVENT)
    event["user"]["name"] = "Synthetic\nReviewer\tΩ"
    event["source"]["site"] = "Synthetic\r\nSite"
    add("audit-whitespace-preservation", "audit", "read", event, "valid",
        preserve=[["agent", 2, "name", event["user"]["name"]], ["source", "site", event["source"]["site"]]])
    for field in ["id", "text", "extension"]:
        value = {"id": "synthetic-event", "text": {"status": "generated", "div": '<div xmlns="http://www.w3.org/1999/xhtml">Synthetic</div>'},
                 "extension": [{"url": "https://example.org/synthetic-extension", "valueString": "synthetic"}]}[field]
        add("audit-common-" + field, "audit", "read", {**EVENT, field: value}, "valid")
    for kind, subtype in [("read", "vread"), ("update", "patch"), ("query", "search-system")]:
        event = copy.deepcopy(EVENT)
        if kind == "query":
            del event["data"]
            event["query"] = "GET /?name=synthetic"
        event["subtype"] = subtype
        add("override-" + kind, "audit", kind, event, "valid", subtype=subtype)
    provenance = {"target": ["Observation/synthetic"], "recorded": EVENT["recorded"], "agent": [{"who": {"reference": "Practitioner/synthetic"}}]}
    add("provenance-control", "lower", BASE + "Provenance", provenance, "valid")
    signature = [{"type": [{"system": "urn:iso-astm:E1762-95:2013", "code": "1.2.840.10065.1.12.1.1"}],
                  "when": EVENT["recorded"], "who": {"reference": "Practitioner/synthetic"}, "data": "c3ludGhldGlj"}]
    add("provenance-signature", "lower", BASE + "Provenance", {**provenance, "signature": signature}, "valid", preserve=[["signature", signature]])
    add("provenance-policy", "lower", BASE + "Provenance", {**provenance, "policy": ["https://example.org/policy"]}, "valid")
    failed = {"type": {"system": "http://terminology.hl7.org/CodeSystem/audit-event-type", "code": "rest"}, "action": "R", "recorded": EVENT["recorded"],
              "outcome": "4", "outcomeDesc": "Synthetic failure\nSecond line\tΩ", "agent": [{"who": {"reference": "Device/synthetic"}, "requestor": True}],
              "source": {"observer": {"reference": "Device/synthetic"}}}
    add("base-audit-failure", "lower", BASE + "AuditEvent", failed, "valid", preserve=[["outcomeDesc", failed["outcomeDesc"]]])
    add("base-audit-sev1", "lower", BASE + "AuditEvent", {**failed, "entity": [{"name": "Synthetic name", "query": "c3ludGhldGlj"}]}, "invalid")
    return out

def previous_inspect(case, result):
    failures = []
    actual = "error" if "error" in result else "valid" if result["validation"]["valid"] else "invalid"
    if actual != case["expect"]:
        failures.append(f"verdict: expected {case['expect']}, got {actual}")
    if "resource" not in result:
        return failures
    resource = result["resource"]
    for field, actual in [("action", resource.get("action")), ("profile", resource.get("meta", {}).get("profile", [None])[0]),
                          ("agents", len(resource.get("agent", [])))]:
        if field in case and actual != case[field]:
            failures.append(field + " differs")
    if case.get("query_bytes") is not None:
        actual = next(e["query"] for e in resource["entity"] if "query" in e)
        if base64.b64decode(actual, validate=True) != case["query_bytes"].encode():
            failures.append("query bytes changed")
    if "network_type" in case and resource["agent"][0]["network"]["type"] != case["network_type"]:
        failures.append("network type differs")
    if "subtype" in case and resource["subtype"][0]["code"] != case["subtype"]:
        failures.append("subtype override lost")
    for path in case.get("preserve", []):
        actual = resource
        try:
            for key in path[:-1]:
                actual = actual[key]
        except (KeyError, IndexError, TypeError):
            actual = None
        if actual != path[-1]:
            failures.append("value changed or missing: " + repr(path[:-1]))
    return failures



def cases():
    out = [c for c in previous_cases() if c['operation'] == 'audit']
    def add(id, event, kind='read', expect='error', **checks):
        out.append(dict(id=id,operation='audit',parameter=kind,inputs=copy.deepcopy(event),expect=expect,**checks))
    def change(path, value, kind='read', **checks):
        event=copy.deepcopy(EVENT)
        if 'query' in kind:
            del event['data'];event['query']='GET /Patient?name=synthetic'
        if kind.startswith('patient-'):event['patient']='Patient/synthetic'
        parent=event;bits=path.split('.')
        for bit in bits[:-1]:parent=parent[bit]
        parent[bits[-1]]=value
        add('shape-'+kind+'-'+path+'-'+str(len(out)),event,kind,**checks)
    # Additional reachable Missing / Unexpected / Malformed branches.
    event=copy.deepcopy(EVENT);del event['user']['who'];add('missing-user.who',event)
    event=copy.deepcopy(EVENT);add('missing-patient',event,'patient-read')
    event=copy.deepcopy(EVENT);del event['data'];add('missing-query',event,'query')
    event=copy.deepcopy(EVENT);event['query']='GET /Patient';add('unexpected-data',event,'query')
    for key,value,kind in [('patient',17,'patient-read'),('query',17,'query'),('source.type',17,'read'),
                           ('user.network',17,'read'),('user.type',17,'create'),('user.role',17,'read'),
                           ('user.purposeOfUse',17,'read'),('subtype',17,'read'),
                           ('client.requestor','yes','read'),('server.requestor','yes','read'),('user.requestor','yes','read')]:
        change(key,value,kind)
    for kind in ['create','delete']:
        change('subtype','read',kind)
    change('user.type','AUT','read')
    change('data',{'what':'Observation/synthetic','role':'4'},'read')
    change('data',{'what':17},'create')
    change('data',{'what':'Observation/synthetic','role':17},'create')
    for parent in ['client','server','user','source']:
        change(parent+'.unrecognised',True)
    change('data',{'what':'Observation/synthetic','unrecognised':True},'create')
    for index,instant in enumerate(['2026','2026-09-17','2026-09-17T10:00:00','not-a-date']):
        event=copy.deepcopy(EVENT);event['recorded']=instant;add('recorded-format-'+str(index),event)
    event=copy.deepcopy(EVENT);event['client']['network']='443'
    add('network-bare-port',event,expect='valid',network_type='1')
    for actor in ['client','server','user']:
        event=copy.deepcopy(EVENT);event[actor]['who']={'display':'Synthetic actor Ω'}
        index=['client','server','user'].index(actor)
        add('who-object-'+actor,event,expect='valid',preserve=[['agent',index,'who',{'display':'Synthetic actor Ω'}]])
    for form,role in [('string','CST'),('object',{'coding':[{'system':'http://terminology.hl7.org/CodeSystem/v3-ParticipationType','code':'CST','display':'Synthetic label'}]})]:
        event=copy.deepcopy(EVENT);event['user']['type']=role
        checks=[['agent',2,'type','coding',0,'code','CST']]
        add('user-type-'+form,event,'create','valid',preserve=checks)
    for form,role in [('string','1'),('object',{'system':'http://terminology.hl7.org/CodeSystem/object-role','code':'1','display':'Synthetic label'})]:
        event=copy.deepcopy(EVENT);event['data']={'what':'Observation/synthetic','role':role}
        add('data-role-excluded-'+form,event,'create','invalid',data_role='1')
    for key in ['role','purposeOfUse']:
        for form,value in [('object',{'text':'Synthetic purpose Ω'}),('array',[{'text':'Synthetic purpose Ω'}])]:
            event=copy.deepcopy(EVENT);event['user'][key]=value
            add('user-'+key+'-'+form,event,expect='valid',preserve=[['agent',2,key,value if isinstance(value,list) else [value]]])
    for form,value in [('object',{'system':'http://terminology.hl7.org/CodeSystem/security-source-type','code':'4'}),('array',[{'system':'http://terminology.hl7.org/CodeSystem/security-source-type','code':'4'}])]:
        event=copy.deepcopy(EVENT);event['source']['type']=value
        add('source-type-'+form,event,expect='valid',preserve=[['source','type',value if isinstance(value,list) else [value]]])
    event=copy.deepcopy(EVENT);event['client']['network']={'address':'synthetic-address','type':'3'}
    add('network-object-override',event,expect='valid',preserve=[['agent',0,'network',event['client']['network']]])
    event=copy.deepcopy(EVENT);event['subtype']={'system':'http://hl7.org/fhir/restful-interaction','code':'vread','display':'Synthetic label'}
    add('subtype-object-override',event,expect='valid',preserve=[['subtype',0,event['subtype']]])
    for case in list(out):
        if case['id'] in ['user-type-object','subtype-object-override']:
            # Preserve the incorrect-display witnesses as failing conformance tests.
            case['expect']='invalid'
            positive=copy.deepcopy(case);positive['id']+='-correct-display';positive['expect']='valid'
            if positive['id'].startswith('user-type'):
                positive['inputs']['user']['type']['coding'][0]['display']='custodian'
            else:
                positive['inputs']['subtype']['display']='vread'
                positive['preserve'][0][-1]['display']='vread'
            out.append(positive)
    event=copy.deepcopy(EVENT);event['data']={'what':'Observation/synthetic','role':'3'}
    add('data-role-valid-override',event,'create','valid',data_role='3')
    # Explicit error category checks for every reachable public rejection field.
    for case in out:
        if case['expect']=='error' and case['id'].startswith('missing-'):
            case['error_text']="'"+case['id'][8:]+"' is required"
    return out


def inspect(case,result):
    failures=previous_inspect(case,result)
    if 'error_text' in case and result.get('error')!=case['error_text']:
        failures.append('wrong error field/category')
    if 'resource' not in result:return failures
    resource=result['resource']
    if 'observed_who' in case:
        expected={'reference' if '/' in case['observed_who'] else 'display':case['observed_who']}
        if resource['agent'][0].get('who')!=expected:failures.append('who string changed')
    if 'data_role' in case:
        entity=next((e for e in resource.get('entity',[]) if e.get('what',{}).get('reference')=='Observation/synthetic'),{})
        if entity.get('role',{}).get('code')!=case['data_role']:failures.append('data role override lost')
    if case['id'].endswith('-user') or case['id'].endswith('-no-user'):
        if resource.get('outcome')!='0':failures.append('successful-event outcome changed')
        if resource.get('type',{}).get('code')!='rest':failures.append('REST type missing')
        kind=case['parameter'].removeprefix('patient-')
        codes={'read':('110152','110153'),'query':('110153','110152'),'create':('110153','110152'),'update':('110153','110152'),'delete':('110150','custodian')}[kind]
        for index,code in enumerate(codes):
            if resource['agent'][index].get('type',{}).get('coding',[{}])[0].get('code')!=code:failures.append('agent type fixed leaf changed')
    return failures

