import { useEffect } from "react";
import { AppLayout } from "./components/layout/AppLayout";
import { useGatewayStore } from "./lib/store";
import "./lib/theme";

export default function App() {
  const initGateway = useGatewayStore((s) => s.init);

  useEffect(() => {
    initGateway();
  }, [initGateway]);

  return <AppLayout />;
}
