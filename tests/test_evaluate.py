"""Counterexamples for evaluation verdicts, provenance and offline result retrieval."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import signal
import tempfile
import unittest
from contextlib import redirect_stdout, redirect_stderr
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/evaluate.py'
spec = importlib.util.spec_from_file_location('evaluate', SCRIPT)
subject = importlib.util.module_from_spec(spec)
spec.loader.exec_module(subject)


def suite():
    return dict(name='synthetic',origin='synthetic',limitations=['Synthetic only'],
                documents={'personal/example.md':'# Example'},cases=[dict(id='one',question='Is the contract met?',category='retrieval',
                steps=[{'op':'read','scope':'personal','path':'example.md'}],
                checks=[{'pointer':'/observations/0/ok','op':'equals','value':True}])])


def report(identity='20261104T100000Z-aaaaaaaaaaaa', passed=True):
    cases = subject.grade(suite(),{'cases':[dict(id='one',observations=[{'ok':passed}],duration_ms=1.0,database_calls=3)]})
    return dict(id=identity,recorded_at='2026-11-04T10:00:00+00:00',suite='synthetic',origin='synthetic',
                execution='actual-engine',bindings=dict(suite_digest='a'*64,grader_digest='b'*64,probe_digest='c'*64,
                source_digest='d'*64,revision='e'*40,environment={'postgres_image':'test'}),cases=cases,
                summary=subject.summary(cases),status='pass' if passed else 'fail',error=None,
                elapsed_ms=2,model_calls=0,llm_usage=None,cleanup={'container_removed':True},limitations=['Synthetic only'])


class EvaluationContracts(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()

    def test_suite_requires_explicit_synthetic_bounded_safe_inputs(self):
        for change in ('origin','path','operation','duplicate'):
            value = suite()
            if change == 'origin': value['origin'] = 'real'
            if change == 'path': value['documents'] = {'personal/../../outside.md':'bad'}
            if change == 'operation': value['cases'][0]['steps'] = [{'op':'export'}]
            if change == 'duplicate': value['cases'].append(copy.deepcopy(value['cases'][0]))
            path = self.root/'suite.json'; path.write_text(json.dumps(value))
            with self.subTest(change=change), self.assertRaises(subject.EvaluationError):
                subject.load_suite(path)

    def test_missing_duplicate_and_truncated_cases_cannot_pass(self):
        value = suite()
        results = subject.grade(value,{'cases':[]})
        self.assertEqual(subject.summary(results),dict(requested=1,evaluated=0,passed=0,failed=0,missing=1))
        row = dict(id='one',observations=[{'ok':True}],duration_ms=1,database_calls=1)
        for rows in ([row,row],[{**row,'observations':[]}],[{**row,'id':'unknown'}]):
            with self.assertRaises(subject.EvaluationError):subject.grade(value,{'cases':rows})

    def test_boolean_and_number_are_not_interchangeable(self):
        cases = subject.grade(suite(),{'cases':[dict(id='one',observations=[{'ok':1}],duration_ms=1,database_calls=1)]})
        self.assertEqual(cases[0]['status'],'fail')

    def test_required_materials_checks_each_original_body(self):
        default = subject.load_suite(subject.SUITE)
        case = next(c for c in default['cases'] if c['id'] == 'required-materials')
        documents = [{'path':path,'body':default['documents'][path].split('---\n',2)[-1].strip()}
                     for path in ['personal/api.md','personal/metrics.md','personal/ui.md']]
        def verdict(items):
            row = dict(id=case['id'],observations=[{'ok':True,'value':{'documents':items}}],duration_ms=1,database_calls=1)
            return subject.grade({**default,'cases':[case]},{'cases':[row]})[0]['status']
        self.assertEqual(verdict(documents),'pass')
        for index in range(len(documents)):
            for replacement in ('','unrelated content'):
                with self.subTest(path=documents[index]['path'],body=replacement):
                    changed = copy.deepcopy(documents);changed[index]['body'] = replacement
                    self.assertEqual(verdict(changed),'fail')

    def readiness_run(self, interruption=None):
        probe_started, attempts = False, 0
        def execute(argv,**_kwargs):
            nonlocal probe_started, attempts
            if argv[:2] == ['docker','exec']:
                attempts += 1
                if attempts == 1:
                    if interruption is not None: signal.raise_signal(interruption)
                    else: raise subject.EvaluationError('database not ready')
            if argv[:2] == ['docker','port']: return b'127.0.0.1:5555'
            if argv[0] == 'cargo':
                probe_started = True
                return b'{"cases":[]}'
            return b''
        with subject.interruption(), patch.object(subject,'command',side_effect=execute), patch.object(subject,'source_state',return_value={'revision':'a'*40,'source_digest':'b'*64}):
            value = subject.run(self.root,subject.SUITE)
        return value, probe_started

    def test_readiness_interruptions_stop_before_probe_and_are_saved(self):
        for signum in (signal.SIGINT,signal.SIGTERM):
            with self.subTest(signal=signum):
                value, started = self.readiness_run(signum)
                self.assertFalse(started)
                self.assertEqual(value['error'],'Evaluation interrupted')
                self.assertEqual(value['status'],'error')
                self.assertEqual(value['summary']['missing'],value['summary']['requested'])
                self.assertTrue(value['cleanup']['container_removed'])
                self.assertEqual(subject.load_report(self.root/(value['id']+'.json')),value)

    def test_readiness_still_retries_temporary_failure(self):
        value, started = self.readiness_run()
        self.assertTrue(started)
        self.assertIsNone(value['error'])
        self.assertTrue(value['cleanup']['container_removed'])

    def test_saved_checksum_and_verdict_corruption_are_rejected(self):
        value = report();path = subject.save(self.root,value)
        self.assertEqual(subject.load_report(path),value)
        envelope = json.loads(path.read_text()); envelope['report']['status']='fail';path.write_text(json.dumps(envelope))
        with self.assertRaisesRegex(subject.EvaluationError,'integrity'):subject.load_report(path)
        value['cases'][0]['checks'][0]['observed']=False
        path.write_bytes(subject.encoded({'report':value,'sha256':subject.sha(subject.encoded(value))}))
        with self.assertRaisesRegex(subject.EvaluationError,'verdict'):subject.load_report(path)

    def test_run_history_is_immutable_and_queries_are_offline(self):
        value = report();path = subject.save(self.root,value);before=path.read_bytes()
        with self.assertRaises(subject.EvaluationError):subject.save(self.root,value)
        with patch.object(subject,'command',side_effect=AssertionError('query executed a command')):
            with redirect_stdout(io.StringIO()) as out:
                self.assertEqual(subject.main(['--results',str(self.root),'history']),0)
            self.assertEqual(json.loads(out.getvalue())[0]['id'],value['id'])
            with patch.object(subject,'applicability',return_value={'status':'stale'}):
                with redirect_stdout(io.StringIO()) as out:
                    self.assertEqual(subject.main(['--results',str(self.root),'latest']),0)
                self.assertEqual(json.loads(out.getvalue())['comparison']['status'],'baseline')
        self.assertEqual(path.read_bytes(),before)

    def test_readonly_missing_directory_does_not_create_state(self):
        root=self.root/'absent'
        with redirect_stderr(io.StringIO()):
            self.assertEqual(subject.main(['--results',str(root),'latest']),2)
        self.assertFalse(root.exists())

    def test_regressions_fixes_incomparable_and_missing_are_distinct(self):
        before=report();after=report('20261104T110000Z-bbbbbbbbbbbb',False)
        self.assertEqual(subject.compare(before,after)['status'],'regressed')
        self.assertEqual(subject.compare(after,before)['status'],'improved')
        after['bindings']['suite_digest']='f'*64
        self.assertEqual(subject.compare(before,after)['status'],'incomparable')
        after['bindings']=copy.deepcopy(before['bindings']);after['error']='execution failed'
        self.assertEqual(subject.compare(before,after)['status'],'inconclusive')
        with self.assertRaises(subject.EvaluationError):subject.compare(before,before)

    def test_changed_source_is_stale_even_if_git_revision_is_unchanged(self):
        value=report();value['bindings']['suite_digest']=subject.sha(subject.encoded(suite()))
        value['bindings']['grader_digest']=subject.sha(SCRIPT.read_bytes())
        value['bindings']['probe_digest']=subject.sha((subject.ROOT/'examples/evaluation_probe.rs').read_bytes())
        with patch.object(subject,'load_suite',return_value=suite()),patch.object(subject,'source_state',return_value={'revision':'e'*40,'source_digest':'f'*64}):
            result=subject.applicability(value,self.root/'unused')
        self.assertEqual(result['status'],'stale');self.assertEqual(result['changed'],['source_digest'])

    def test_failed_execution_is_saved_with_missing_cases(self):
        path=self.root/'suite.json';path.write_text(json.dumps(suite()))
        def execute(argv,**_kwargs):
            if argv[:2]==['docker','run']:raise subject.EvaluationError('synthetic startup failure')
            return b''
        with patch.object(subject,'command',side_effect=execute),patch.object(subject,'source_state',return_value={'revision':'a'*40,'source_digest':'b'*64}):
            value=subject.run(self.root,path)
        self.assertEqual(value['status'],'error');self.assertEqual(value['summary']['missing'],1)
        self.assertEqual(subject.load_report(self.root/(value['id']+'.json')),value)

    def test_symlinked_result_root_is_rejected(self):
        target=self.root/'real';target.mkdir();link=self.root/'link';link.symlink_to(target,target_is_directory=True)
        with self.assertRaises(subject.EvaluationError):subject.result_directory(link)

    def test_source_fingerprint_excludes_generated_vendor_builds(self):
        source=self.root/'src/new.rs';source.parent.mkdir();source.write_text('new engine source')
        generated=self.root/'vendor/parser/target/build/generated.rs'
        names=b'src/new.rs\nvendor/parser/target/build/generated.rs\nvendor/parser/target/object.o\n'
        def execute(argv,**_kwargs):
            return names if argv[1]=='ls-files' else b'revision'
        with patch.object(subject,'ROOT',self.root),patch.object(subject,'command',side_effect=execute):
            before=subject.source_state()
            generated.parent.mkdir(parents=True);generated.write_text('compiler output')
            self.assertEqual(subject.source_state(),before)
            source.write_text('changed untracked engine source')
            self.assertNotEqual(subject.source_state()['source_digest'],before['source_digest'])

    def test_history_uses_recorded_time_instead_of_uuid_order(self):
        later=report('20261104T100000Z-aaaaaaaaaaaa');later['recorded_at']='2026-11-04T10:00:00.900000+00:00'
        earlier=report('20261104T100000Z-bbbbbbbbbbbb');earlier['recorded_at']='2026-11-04T10:00:00.100000+00:00'
        subject.save(self.root,later);subject.save(self.root,earlier)
        self.assertEqual([r['id'] for r in subject.reports(self.root)],[earlier['id'],later['id']])

    def test_toolchain_preparation_failure_is_not_hidden_by_previous_pass(self):
        results=self.root/'results';results.mkdir()
        previous=report();previous['recorded_at']='2000-01-01T00:00:00+00:00'
        subject.save(results,previous);path=self.root/'suite.json';path.write_text(json.dumps(suite()))
        def execute(argv,**_kwargs):
            self.assertNotEqual(argv[0],'docker')
            raise subject.EvaluationError('rustc unavailable')
        with patch.object(subject,'command',side_effect=execute),patch.object(subject,'source_state',return_value={'revision':'a'*40,'source_digest':'b'*64}):
            value=subject.run(results,path)
        self.assertEqual(value['status'],'error');self.assertEqual(value['summary']['missing'],1)
        self.assertEqual(subject.reports(results)[-1]['id'],value['id'])

    def test_probe_and_grader_drift_during_execution_cannot_be_saved_as_pass(self):
        for changed in ('probe','grader'):
            with self.subTest(changed=changed),tempfile.TemporaryDirectory(dir=self.root) as directory:
                root=Path(directory);(root/'examples').mkdir();probe=root/'examples/evaluation_probe.rs';probe.write_text('probe')
                grader=root/'grader.py';grader.write_text('grader');path=root/'suite.json';path.write_text(json.dumps(suite()))
                def execute(argv,**_kwargs):
                    if argv[0]=='cargo':
                        (probe if changed=='probe' else grader).write_text('changed')
                        return subject.encoded({'cases':[dict(id='one',observations=[{'ok':True}],duration_ms=1,database_calls=1)]})
                    if argv[:2]==['docker','port']:return b'127.0.0.1:5555'
                    return b''
                with patch.object(subject,'ROOT',root),patch.object(subject,'__file__',str(grader)),patch.object(subject,'command',side_effect=execute),patch.object(subject,'source_state',return_value={'revision':'a'*40,'source_digest':'b'*64}):
                    value=subject.run(root,path)
                self.assertEqual(value['status'],'error');self.assertIn('inputs changed',value['error'])


if __name__ == '__main__':unittest.main()
