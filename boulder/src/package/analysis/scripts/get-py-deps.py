#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2024 AerynOS Developers
# SPDX-License-Identifier: MPL-2.0

import argparse
import os
import sys
import unittest
from importlib.metadata import Distribution

from packaging.requirements import Requirement
from packaging.markers import Marker

parser = argparse.ArgumentParser()
parser.add_argument("location", type=str, help="Location of .egg-info or .dist-info directory")

@staticmethod
def get_dependencies(dependencies: list, env=None) -> list:
    sanitized = []
    for dep in dependencies:
        req = Requirement(dep)
        if not req.extras:
            if req.marker:
                mark = req.marker
                if mark.evaluate(environment=env) is True:
                    sanitized.append(req.name)
                    continue
            else:
                sanitized.append(req.name)
    return sanitized

def usage(msg=None, ex=1):
    if msg:
        print(msg, file=sys.stderr)
    else:
        parser.print_help()
    sys.exit(ex)

if __name__ == "__main__":
    args = parser.parse_args()

    if not os.path.exists(args.location):
        usage()

    if not (os.path.exists(os.path.join(args.location, 'METADATA')) or os.path.exists(os.path.join(args.location, 'PKG-INFO'))):
        usage("Unable to find PKG-INFO or METADATA files in specified directory")

    dependencies = Distribution.at(args.location).requires

    if dependencies:
        for dep in get_dependencies(dependencies):
            print(dep)


class Tests(unittest.TestCase):

    def test_basic(self):
        self.assertEqual(get_dependencies(['six']), ['six'])

    def test_empty_list(self):
        self.assertEqual(get_dependencies([]), [])

    def test_excludes_version(self):
        self.assertEqual(get_dependencies(['six>=6.9']), ['six'])

    def test_excludes_extras(self):
        list1 = ['MarkupSafe>=2.0', 'Babel>=2.7; extra == "i18n"']
        self.assertEqual(get_dependencies(list1), ['MarkupSafe'])

    def test_excludes_extras_no_deps(self):
        self.assertEqual(get_dependencies(['Babel>=2.7; extra == "i18n"', 'pytest; extra == "testing"']), [])

    def test_excludes_extras_comprehensive(self):
        list2 = ['MarkupSafe>=0.9.2', 'Babel; extra == "babel"', 'lingua; extra == "lingua"', 'pytest; extra == "testing"']
        self.assertEqual(get_dependencies(list2), ['MarkupSafe'])

    def test_ignore_satisfied_evaluate_markers(self):
        env = {'python_version': '3.11'}
        list3 = ["tomli>=1.2.2; python_version < '3.11'", 'pluggy>=1.0.0']
        self.assertEqual(get_dependencies(list3, env), ['pluggy'])

    def test_included_unsatisified_evaluate_markers(self):
        env = {'python_version': '3.11'}
        list4 = ['pluggy', "tomli>=1.2.2; python_version < '3.12'"]
        self.assertEqual(get_dependencies(list4, env), ['pluggy', 'tomli'])

    def test_comprehensive1(self):
        env = {'python_version': '3.11'}
        list5 = ['pytest; extra == "testing"', 'editables>=0.3', 'packaging>=21.3', 'pathspec>=0.10.1', 'pluggy>=1.0.0', "tomli>=1.2.2; python_version < '3.12'", 'trove-classifiers']
        self.assertEqual(get_dependencies(list5, env), ['editables', 'packaging', 'pathspec', 'pluggy', 'tomli', 'trove-classifiers'])

    def test_comprehensive2(self):
        list6 = ["brotli; implementation_name == 'cpython'", "brotlicffi; implementation_name != 'cpython'", 'certifi', 'mutagen', 'pycryptodomex', 'requests<3,>=2.32.2', 'urllib3<3,>=1.26.17', 'websockets>=12.0', "build; extra == 'build'", "hatchling; extra == 'build'", "pip; extra == 'build'", "setuptools>=71.0.2; extra == 'build'", "wheel; extra == 'build'", "curl-cffi!=0.6.*,<0.8,>=0.5.10; (os_name != 'nt' and implementation_name == 'cpython') and extra == 'curl-cffi'", "curl-cffi==0.5.10; (os_name == 'nt' and implementation_name == 'cpython') and extra == 'curl-cffi'", "pre-commit; extra == 'dev'", "yt-dlp[static-analysis]; extra == 'dev'", "yt-dlp[test]; extra == 'dev'", "py2exe>=0.12; extra == 'py2exe'", "pyinstaller>=6.7.0; extra == 'pyinstaller'", "cffi; extra == 'secretstorage'", "secretstorage; extra == 'secretstorage'", "autopep8~=2.0; extra == 'static-analysis'", "ruff~=0.5.0; extra == 'static-analysis'", "pytest~=8.1; extra == 'test'"]
        self.assertEqual(get_dependencies(list6), ['brotli', 'certifi', 'mutagen', 'pycryptodomex', 'requests', 'urllib3', 'websockets'])
