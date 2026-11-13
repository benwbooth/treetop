{
  description = "treetop - an htop-style system monitor for KDE Plasma (process tree, GPU, containers, tray icon, panel widget)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = import nixpkgs { inherit system; };
        qtModules = (with pkgs.qt6; [ qtbase qtdeclarative qtsvg qtwayland ])
          ++ (with pkgs.kdePackages; [ qqc2-desktop-style plasma-integration kirigami ]);
        qtEnv = pkgs.qt6.env "treetop-qt-env" qtModules;
        # pkexec (polkit) and kill are needed to signal processes owned by
        # other users; the app asks polkit and the desktop agent prompts.
        runtimePath = pkgs.lib.makeBinPath [ pkgs.coreutils pkgs.util-linux ];
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "treetop";
          version = cargoToml.package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config pkgs.qt6.wrapQtAppsHook pkgs.makeWrapper ];
          buildInputs = qtModules;
          preBuild = ''
            export QMAKE="${qtEnv}/bin/qmake"
          '';
          postInstall = ''
            mkdir -p $out/share/plasma/plasmoids $out/share/applications $out/share/icons/hicolor/scalable/apps
            cp -R plasmoid $out/share/plasma/plasmoids/com.benwbooth.treetop
            install -Dm644 resources/treetop.desktop $out/share/applications/com.benwbooth.treetop.desktop
            install -Dm644 resources/treetop.svg $out/share/icons/hicolor/scalable/apps/treetop.svg
          '';
          qtWrapperArgs = [ "--suffix PATH : ${runtimePath}" ];
          meta = {
            description = "htop-style system monitor for KDE Plasma";
            homepage = "https://github.com/benwbooth/treetop";
            license = pkgs.lib.licenses.mit;
            mainProgram = "treetop";
            platforms = pkgs.lib.platforms.linux;
          };
        };

        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [ cargo rustc clippy rustfmt rust-analyzer pkg-config clang ]) ++ qtModules;
          QMAKE = "${qtEnv}/bin/qmake";
          shellHook = ''
            export PATH="${qtEnv}/bin:${qtEnv}/libexec:$PATH"
            # qtbase's setup hook points QMAKE at bare qtbase; cxx-qt needs the
            # merged env so it can find every module's .prl.
            export QMAKE="${qtEnv}/bin/qmake"
            NIX_CFLAGS_LINK="$(printf '%s\n' ''${NIX_CFLAGS_LINK:-} | grep -v '^-L/usr/lib' | tr '\n' ' ')"
            export NIX_CFLAGS_LINK
            export QT_PLUGIN_PATH="${qtEnv}/lib/qt-6/plugins"
            export QML_IMPORT_PATH="${qtEnv}/lib/qt-6/qml"
          '';
        };
      });
}
