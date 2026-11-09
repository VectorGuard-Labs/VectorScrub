import ErrorBoundary from "@/components/ErrorBoundary";

/**
 * Layout for dashboard routes with error boundary protection
 */
export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return <ErrorBoundary>{children}</ErrorBoundary>;
}
