import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import { ThemeProvider } from "@/components/layout/ThemeProvider";
import { AppShell } from "@/components/layout/AppShell";
import { ToastProvider } from "@/components/ui/Toast";

import { HomePage } from "@/routes/HomePage";
import { SettingsPage } from "@/routes/SettingsPage";
import { AboutPage } from "@/routes/AboutPage";

import { MergePage } from "@/features/merge/MergePage";
import { SplitPage } from "@/features/split/SplitPage";
import { ReorderPage } from "@/features/reorder/ReorderPage";
import { CompressPage } from "@/features/compress/CompressPage";
import { ImagesPage } from "@/features/images/ImagesPage";
import { RepairPage } from "@/features/repair/RepairPage";
import { ProtectPage } from "@/features/protect/ProtectPage";
import { OfficeToPdfPage } from "@/features/office/OfficeToPdfPage";
import { PdfToOfficePage } from "@/features/office/PdfToOfficePage";
import { OcrPage } from "@/features/ocr/OcrPage";
import { PageNumbersPage } from "@/features/page-numbers/PageNumbersPage";
import { UnlockPage } from "@/features/unlock/UnlockPage";
import { WatermarkPage } from "@/features/watermark/WatermarkPage";
import { CropPage } from "@/features/crop/CropPage";
import { PdfaPage } from "@/features/pdfa/PdfaPage";
import { ComparePage } from "@/features/compare/ComparePage";
import { StampPage } from "@/features/stamp/StampPage";
import { PosterPage } from "@/features/poster/PosterPage";
import { NupPage } from "@/features/nup/NupPage";
import { BlankPage } from "@/features/blank/BlankPage";
import { MetadataPage } from "@/features/metadata/MetadataPage";
import { TextExportPage } from "@/features/text-export/TextExportPage";
import { EditorSandboxPage } from "@/features/editor-sandbox/EditorSandboxPage";

export function App() {
  return (
    <ThemeProvider>
      <ToastProvider>
        <HashRouter>
          <Routes>
            <Route element={<AppShell />}>
              <Route path="/" element={<HomePage />} />
              <Route path="/tools/merge" element={<MergePage />} />
              <Route path="/tools/split" element={<SplitPage />} />
              <Route path="/tools/reorder" element={<ReorderPage />} />
              <Route path="/tools/compress" element={<CompressPage />} />
              <Route path="/tools/office-to-pdf" element={<OfficeToPdfPage />} />
              <Route path="/tools/pdf-to-office" element={<PdfToOfficePage />} />
              <Route path="/tools/images" element={<ImagesPage />} />
              <Route path="/tools/repair" element={<RepairPage />} />
              <Route path="/tools/protect" element={<ProtectPage />} />
              <Route path="/tools/ocr" element={<OcrPage />} />
              <Route path="/tools/page-numbers" element={<PageNumbersPage />} />
              <Route path="/tools/unlock" element={<UnlockPage />} />
              <Route path="/tools/watermark" element={<WatermarkPage />} />
              <Route path="/tools/crop" element={<CropPage />} />
              <Route path="/tools/pdfa" element={<PdfaPage />} />
              <Route path="/tools/compare" element={<ComparePage />} />
              <Route path="/tools/stamp" element={<StampPage />} />
              <Route path="/tools/poster" element={<PosterPage />} />
              <Route path="/tools/nup" element={<NupPage />} />
              <Route path="/tools/blank-pages" element={<BlankPage />} />
              <Route path="/tools/metadata" element={<MetadataPage />} />
              <Route path="/tools/text-export" element={<TextExportPage />} />
              <Route path="/dev/editor-canvas" element={<EditorSandboxPage />} />
              <Route path="/settings" element={<SettingsPage />} />
              <Route path="/about" element={<AboutPage />} />
              <Route path="*" element={<Navigate to="/" replace />} />
            </Route>
          </Routes>
        </HashRouter>
      </ToastProvider>
    </ThemeProvider>
  );
}
