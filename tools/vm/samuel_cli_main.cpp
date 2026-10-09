// Headless front end for SAMUEL's core (the Qt GUI isn't scriptable).
//   samuel-cli <file.resources> list [substring...]
//   samuel-cli <file.resources> export <outdir> <exact resource name>...
#include "../source/core/SAMUEL.h"
#include <cstdio>

int main(int argc, char** argv)
{
    if (argc < 3) { std::fprintf(stderr, "usage: samuel-cli <resources> list|export ...\n"); return 2; }
    std::string res = argv[1], cmd = argv[2];
    HAYDEN::GLOBAL_RESOURCES globals;
    HAYDEN::SAMUEL sam;
    if (!sam.Init(res, globals)) { std::fprintf(stderr, "init failed: %s %s\n", sam.GetLastErrorMessage().c_str(), sam.GetLastErrorDetail().c_str()); return 1; }
    sam.LoadResource(res);
    if (sam.HasResourceLoadError()) { std::fprintf(stderr, "load failed: %s\n", sam.GetLastErrorMessage().c_str()); return 1; }
    auto data = sam.GetResourceData();
    if (cmd == "list") {
        for (auto& e : data) {
            bool ok = true;
            for (int i = 3; i < argc; i++) if (e.Name.find(argv[i]) == std::string::npos) ok = false;
            if (ok) std::printf("%s\t%s\t%u\n", e.Name.c_str(), e.Type.c_str(), e.Version);
        }
        return 0;
    }
    if (cmd == "export" && argc >= 5) {
        std::vector<std::vector<std::string>> rows;
        for (int i = 4; i < argc; i++)
            for (auto& e : data)
                if (e.Name == argv[i]) rows.push_back({ e.Name, e.Type, std::to_string(e.Version) });
        std::printf("exporting %zu entries\n", rows.size());
        bool ok = sam.ExportFiles(fs::path(argv[3]), rows);
        std::printf("export %s\n", ok ? "ok" : "failed");
        return ok ? 0 : 1;
    }
    if (cmd == "raw" && argc >= 5) {
        // raw <outdir> <exact name>... : dump decompressed embedded bytes (e.g. font metrics)
        HAYDEN::ResourceFileReader reader(res);
        for (int i = 4; i < argc; i++)
            for (auto& e : data)
                if (e.Name == argv[i]) {
                    auto bytes = reader.GetEmbeddedFileHeader(res, e.DataOffset, e.DataSize, e.DataSizeUncompressed);
                    std::string fn = e.Name; for (auto& c : fn) if (c == '/' || c == ' ' || c == '$' || c == ':') c = '_';
                    fs::path out = fs::path(argv[3]) / (fn + "." + e.Type + ".bin");
                    std::ofstream(out, std::ios::binary).write((const char*)bytes.data(), bytes.size());
                    std::printf("%s -> %zu bytes (comp %llu)\n", e.Name.c_str(), bytes.size(), (unsigned long long)e.CompressionMode);
                }
        return 0;
    }
    return 2;
}
