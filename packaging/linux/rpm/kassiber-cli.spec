%global debug_package %{nil}

Name:           kassiber-cli
Version:        %{kassiber_version}
Release:        %{kassiber_release}
Summary:        Local-first Bitcoin accounting CLI
License:        MIT
URL:            https://github.com/bitcoinaustria/kassiber
Source0:        kassiber
Source1:        LICENSE
BuildArch:      %{kassiber_arch}

# The Rust binary links only glibc and libgcc_s; rpmbuild's automatic ELF
# requirements name both.
Requires:       glibc >= 2.35
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
  %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/kassiber

%changelog
* Thu Jan 01 1970 Bitcoin Austria <office@bitcoin-austria.at> - 0-1
- Package the Kassiber CLI release artifact.
