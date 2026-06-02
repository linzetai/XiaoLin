import { useEffect } from "react";
import { LucideProvider } from "lucide-react";
import { AppLayout } from "./components/layout/AppLayout";
import { ContextMenuProvider } from "./components/common/ContextMenu";
import { TooltipProvider } from "./components/common/Tooltip";
import { ImageLightbox } from "./components/common/ImageLightbox";
import { useGatewayStore } from "./lib/store";
import "./lib/theme";

export default function App() {
  const initGateway = useGatewayStore((s) => s.init);

  useEffect(() => {
    initGateway();
  }, [initGateway]);

  return (
    <LucideProvider absoluteStrokeWidth>
      <AppLayout />
      <ContextMenuProvider />
      <TooltipProvider />
      <ImageLightbox />
    </LucideProvider>
  );
}
