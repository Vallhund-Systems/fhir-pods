#!/usr/bin/env python3
"""Fixed synthetic slot-shape, shorthand, preservation and conflict examples."""
import copy
AU='http://hl7.org.au/fhir/core/StructureDefinition/'
US='http://hl7.org/fhir/us/core/StructureDefinition/'
BASE='http://hl7.org/fhir/StructureDefinition/'
BALP='https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.'
TIME='2026-09-17T10:00:00Z'
HR={'status':'final','subject':'Patient/synthetic','effectiveDateTime':TIME,'valueQuantity.value':72,'valueQuantity.unit':'beats/minute'}
PT={'identifier':[{'system':'https://example.org/identifiers','value':'synthetic'}],'name':[{'family':'Synthetic','given':['Example']}],'gender':'unknown','birthDate':'1980-01-01'}
PROV={'target':['Observation/synthetic'],'recorded':TIME,'agent':[{'who':{'reference':'Practitioner/synthetic'}}]}
DR={'status':'final','category':{'text':'Synthetic category'},'code':{'text':'Synthetic report'},'subject':'Patient/synthetic','effectiveDateTime':TIME}
MED={'status':'active','intent':'order','medicationCodeableConcept':{'text':'Synthetic medicine'},'subject':'Patient/synthetic','requester':'Practitioner/synthetic'}
CONDITION={'clinicalStatus':{'system':'http://terminology.hl7.org/CodeSystem/condition-clinical','code':'active'},'category':{'system':'http://terminology.hl7.org/CodeSystem/condition-category','code':'problem-list-item'},'code':{'text':'Synthetic condition'},'subject':'Patient/synthetic'}
QUERY={'recorded':TIME,'agent:client.who':'Device/client','agent:client.network':{'address':'192.0.2.1','type':'2'},'agent:client.requestor':False,'agent:server.who':'Device/server','agent:server.network':{'address':'192.0.2.2','type':'2'},'agent:server.requestor':False,'source':{'observer':{'reference':'Device/audit'}},'entity:query.query':'c3ludGhldGlj','action':'E','subtype':{'system':'http://hl7.org/fhir/restful-interaction','code':'search-type'}}
NARRATIVE={'status':'generated','div':'<div xmlns="http://www.w3.org/1999/xhtml">Synthetic narrative</div>'}
SIG={'type':[{'system':'urn:iso-astm:E1762-95:2013','code':'1.2.840.10065.1.12.1.1'}],'when':TIME,'who':{'reference':'Practitioner/synthetic'},'data':'c3ludGhldGlj'}


def cases():
    out=[]
    def add(id,profile,inputs,expect='valid',**checks):
        out.append(dict(id=id,operation='lower',parameter=profile,inputs=copy.deepcopy(inputs),expect=expect,**checks))
    def slot(ty,profile,seed,key,value,path=None):
        inputs=copy.deepcopy(seed);inputs[key]=value
        add('full-'+ty,profile,inputs,slot_type=ty,preserve=[[*(path or key.split('.')),value]])
        inputs[key]=False if ty!='boolean' else 'true'
        add('wrong-kind-'+ty,profile,inputs,'invalid',slot_type=ty,issue={'path':key,'code':'value'})
    # Each current primitive/complex slot type, plus a concrete choice and carrier.
    slot('code',AU+'au-core-heartrate',HR,'status','final')
    slot('dateTime',AU+'au-core-heartrate',HR,'effectiveDateTime',TIME)
    slot('decimal',AU+'au-core-heartrate',HR,'valueQuantity.value',72,['valueQuantity','value'])
    slot('string',AU+'au-core-heartrate',HR,'valueQuantity.unit','beats/minute',['valueQuantity','unit'])
    slot('date',AU+'au-core-patient',PT,'birthDate','1980-01-01')
    slot('boolean',US+'us-core-organization',{'name':'Synthetic organization','active':True},'active',True)
    slot('instant',BASE+'Provenance',PROV,'recorded',TIME)
    slot('uri',BASE+'Provenance',PROV,'policy','https://example.org/synthetic-policy',['policy',0])
    slot('canonical',US+'us-core-questionnaireresponse',{'status':'completed','subject':'Patient/synthetic','authored':TIME},'questionnaire','https://example.org/Questionnaire/synthetic')
    slot('base64Binary',BALP+'Query',QUERY,'entity:query.query','c3ludGhldGlj',['entity',0,'query'])
    slot('Reference',AU+'au-core-heartrate',HR,'subject',{'reference':'Patient/synthetic'})
    slot('HumanName',AU+'au-core-patient',PT,'name',{'family':'Synthetic','given':['Example']},['name',0])
    slot('Identifier',AU+'au-core-patient',PT,'identifier',{'system':'https://example.org/identifiers','value':'synthetic'},['identifier',0])
    slot('Address',AU+'au-core-patient',PT,'address',{'city':'Synthetic city'},['address',0])
    slot('ContactPoint',AU+'au-core-patient',PT,'telecom',{'system':'email','value':'synthetic@example.org'},['telecom',0])
    slot('Narrative',AU+'au-core-patient',PT,'text',NARRATIVE)
    slot('Signature',BASE+'Provenance',PROV,'signature',SIG,['signature',0])
    slot('Attachment',US+'us-core-diagnosticreport-note',DR,'presentedForm',{'contentType':'text/plain','data':'c3ludGhldGlj'},['presentedForm',0])
    slot('Dosage',US+'us-core-medicationrequest',MED,'dosageInstruction',{'text':'Synthetic instruction'},['dosageInstruction',0])
    slot('Annotation',AU+'au-core-condition',CONDITION,'note',{'text':'Synthetic note Ω'},['note',0])
    period_seed={k:v for k,v in DR.items() if k!='effectiveDateTime'}
    slot('Period',US+'us-core-diagnosticreport-note',period_seed,'effectivePeriod',{'start':TIME})
    slot('CodeableConcept',US+'us-core-diagnosticreport-note',DR,'code',{'text':'Synthetic report'})
    audit={'type':{'system':'http://terminology.hl7.org/CodeSystem/audit-event-type','code':'rest'},'recorded':TIME,'agent':[{'who':{'reference':'Device/synthetic'},'requestor':True}],'source':{'observer':{'reference':'Device/audit'}}}
    slot('Coding',BASE+'AuditEvent',audit,'type',audit['type'])
    slot('BackboneElement',BASE+'Provenance',PROV,'agent',{'who':{'reference':'Practitioner/synthetic'}},['agent',0])
    quantity_seed={k:v for k,v in HR.items() if not k.startswith('valueQuantity.')}
    slot('Ratio',BASE+'Medication',{},'amount',{'numerator':{'value':1},'denominator':{'value':2}})
    slot('Quantity',AU+'au-core-heartrate',quantity_seed,'valueQuantity',{'value':72,'unit':'beats/minute','system':'http://unitsofmeasure.org','code':'/min'})
    add('quantity-shorthand',AU+'au-core-heartrate',{**quantity_seed,'valueQuantity':72,'valueQuantity.unit':'beats/minute'},preserve=[['valueQuantity','value',72]])
    add('reference-shorthand',AU+'au-core-heartrate',HR,preserve=[['subject','reference','Patient/synthetic']])
    add('concept-text-shorthand',US+'us-core-diagnosticreport-note',{**DR,'code':'Synthetic report Ω'},preserve=[['code','text','Synthetic report Ω']])
    add('concept-coding-shorthand',US+'us-core-diagnosticreport-note',{**DR,'code':{'system':'https://example.org/synthetic-codes','code':'synthetic'}},preserve=[['code','coding',0,'code','synthetic']])
    add('choice-concrete',US+'us-core-diagnosticreport-note',DR,slot_type='choice',preserve=[['effectiveDateTime',TIME]])
    add('choice-generic-key',US+'us-core-diagnosticreport-note',{**period_seed,'effective[x]':TIME},'invalid',slot_type='choice')
    add('choice-two-keys',US+'us-core-diagnosticreport-note',{**DR,'effectivePeriod':{'start':TIME}},'invalid')
    for label,patch in [('unknown-key',{'notASlot':'synthetic'}),('fixed-collision',{'valueQuantity.code':'kg'}),('carrier-kind',{'_effectiveDateTime':False})]:
        add(label,AU+'au-core-heartrate',{**HR,**patch},'invalid')
    carrier={'extension':[{'url':'http://hl7.org/fhir/StructureDefinition/data-absent-reason','valueCode':'unknown'}]}
    absent={k:v for k,v in HR.items() if k!='effectiveDateTime'}
    # The computable vs-1 expression, as evaluated by both reference engines,
    # rejects an extension-only date despite the human wording's value guard.
    add('primitive-carrier',AU+'au-core-heartrate',{**absent,'_effectiveDateTime':carrier},'invalid',preserve=[['_effectiveDateTime',carrier]],slot_type='Element',issue={'path':'Observation.effective[x]','code':'invariant','message_contains':'vs-1:'})
    # Optional labelled identifier: whole input and a matching/conflicting child.
    identifier={'value':'8003608166690503'}
    add('whole-slice',AU+'au-core-patient',{**PT,'identifier:ihi':identifier},preserve=[['identifier',1,'value',identifier['value']]])
    add('slice-whole-and-part',AU+'au-core-patient',{**PT,'identifier:ihi':identifier,'identifier:ihi.system':'http://ns.electronichealth.net.au/id/hi/ihi/1.0'})
    add('slice-conflicting-part',AU+'au-core-patient',{**PT,'identifier:ihi':identifier,'identifier:ihi.system':'https://example.org/wrong-system'},'invalid')
    add('deep-optional-gate',AU+'au-core-patient',{**PT,'identifier:ihi.system':'http://ns.electronichealth.net.au/id/hi/ihi/1.0'},'invalid',preserve=[['identifier',1,'system','http://ns.electronichealth.net.au/id/hi/ihi/1.0']])
    add('unknown-slice-part',AU+'au-core-patient',{**PT,'identifier:ihi':identifier,'identifier:ihi.value':'8003608166690503'},'invalid')
    add('fixed-optional-action',BALP+'Query',QUERY,preserve=[['action','E']])
    add('fixed-optional-conflict',BALP+'Query',{**QUERY,'action':'R'},'invalid')
    add('unknown-profile','https://example.org/StructureDefinition/not-compiled',{},'error')
    add('inputs-not-object',AU+'au-core-heartrate',[],'error')
    precise={
        'choice-generic-key':{'path':'effective[x]','code':'value'},
        'choice-two-keys':{'path':'DiagnosticReport.effective[x]','code':'structure','message_contains':'one choice'},
        'unknown-key':{'path':'notASlot','code':'structure'},
        'fixed-collision':{'path':'valueQuantity.code','code':'value','message_contains':'fixed'},
        'carrier-kind':{'path':'_effectiveDateTime','code':'value'},
        'slice-conflicting-part':{'path':'identifier:ihi.system','code':'value','message_contains':'fixed'},
        'deep-optional-gate':{'path':'Patient.identifier:ihi.value','code':'required'},
        'unknown-slice-part':{'path':'identifier:ihi.value','code':'structure'},
        'fixed-optional-conflict':{'path':'action','code':'value','message_contains':'fixed'},
    }
    for case in out:
        if case['id'] in precise:case['issue']=precise[case['id']]
        if case['id']=='unknown-profile':case['error_text']='no compiled profile with that URL'
        if case['id']=='inputs-not-object':case['error_text']='inputs must be a JSON object'
    return out


def inspect(case,result):
    failures=[]
    actual='error' if 'error' in result else 'valid' if result['validation']['valid'] else 'invalid'
    if actual!=case['expect']:failures.append('verdict: expected '+case['expect']+', got '+actual)
    if 'error_text' in case and result.get('error')!=case['error_text']:failures.append('wrong operation error')
    if 'issue' in case:
        wanted=case['issue']
        if not any(i.get('path')==wanted['path'] and i.get('code')==wanted['code'] and i.get('severity') in ['error','fatal'] and wanted.get('message_contains','') in i.get('message','') for i in result.get('validation',{}).get('issues',[])):
            failures.append('missing intended input issue: '+repr(wanted))
    if 'resource' not in result:return failures
    for path in case.get('preserve',[]):
        value=result['resource']
        try:
            for key in path[:-1]:value=value[key]
        except (KeyError,TypeError,IndexError):value=None
        if not preserves(value,path[-1]):failures.append('value changed or missing: '+repr(path[:-1]))
    return failures


def preserves(actual,expected):
    # Lowering may supply fixed leaves and known coding displays; every given
    # value must still be present and unchanged.
    if isinstance(expected,dict):
        return isinstance(actual,dict) and all(k in actual and preserves(actual[k],v) for k,v in expected.items())
    if isinstance(expected,list):
        return isinstance(actual,list) and len(actual)==len(expected) and all(preserves(a,e) for a,e in zip(actual,expected))
    return type(actual)==type(expected) and actual==expected
