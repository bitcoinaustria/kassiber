%global debug_package %{nil}

Name:           kassiber-cli
Version:        %{kassiber_version}
Release:        %{kassiber_release}
Summary:        Local-first Bitcoin accounting CLI
License:        AGPL-3.0-only
URL:            https://github.com/bitcoinaustria/kassiber
Source0:        kassiber
Source1:        install-context.json
Source2:        LICENSE
BuildArch:      %{kassiber_arch}

Requires:       glibc >= 2.35
%if 0%{?suse_version}
Requires:       libz1
%else
Requires:       zlib
%endif
Provides:       kassiber-command
Conflicts:      kassiber

%description
Kassiber is a local-first Bitcoin accounting suite. This package contains the
standalone command-line application without desktop dependencies.

%prep

%build

%install
install -Dpm 0755 %{SOURCE0} %{buildroot}%{_bindir}/kassiber
install -Dpm 0644 %{SOURCE1} \
  %{buildroot}%{_prefix}/lib/kassiber/install-context.json
install -Dpm 0644 %{SOURCE2} \
  %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/kassiber
%{_prefix}/lib/kassiber/install-context.json

%changelog
* Thu Jan 01 1970 Bitcoin Austria <office@bitcoin-austria.at> - 0-1
- Package the frozen Kassiber CLI release artifact.
