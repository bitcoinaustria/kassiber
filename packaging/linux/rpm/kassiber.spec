%global debug_package %{nil}

Name:           kassiber
Version:        %{kassiber_version}
Release:        %{kassiber_release}
Summary:        Local-first Bitcoin accounting desktop application
License:        AGPL-3.0-only
URL:            https://github.com/bitcoinaustria/kassiber
Source0:        kassiber-desktop-rootfs.tar.gz
Source1:        LICENSE
BuildArch:      %{kassiber_arch}

Requires:       glibc >= 2.35
%if 0%{?suse_version}
Requires:       libgtk-3-0
Requires:       libwebkit2gtk-4_1-0
%else
Requires:       gtk3
Requires:       webkit2gtk4.1
%endif
Provides:       kassiber-command
Conflicts:      kassiber-cli

%description
Kassiber is a local-first Bitcoin accounting suite. This package contains the
desktop application and its bundled command-line sidecar.

%prep
%setup -q -c -T
tar -xzf %{SOURCE0}

%build

%install
cp -a . %{buildroot}
install -Dpm 0644 %{SOURCE1} \
  %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/kassiber
%{_bindir}/kassiber-ui
%{_prefix}/lib/Kassiber/
%{_datadir}/applications/Kassiber.desktop
%{_datadir}/icons/hicolor/*/apps/kassiber-ui.png

%changelog
* Thu Jan 01 1970 Bitcoin Austria <office@bitcoin-austria.at> - 0-1
- Package the Kassiber desktop release payload.
