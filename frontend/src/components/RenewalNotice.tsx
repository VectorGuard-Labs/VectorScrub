import Link from "next/link";

/**
 * The automatic-renewal terms of a paid plan, in plain words. It is rendered directly above the
 * button that starts the purchase (and beside the price on the home page), not in a link or a
 * footer: a customer must be able to see that the plan renews, at what price, how often, and how
 * to stop it, at the moment they decide to subscribe.
 *
 * The figures come from the caller, which takes them from the same table as the price shown on
 * the card, so the notice cannot disagree with the price next to it. Keep the wording in step
 * with section 6 of the Terms and with what the Stripe payment link says at checkout.
 */
export default function RenewalNotice({
  name,
  price,
  interval,
  locksPrice = false,
  id,
}: {
  name: string;
  price: string;
  interval: "month" | "year";
  /** True for a plan whose current price is kept for existing members. */
  locksPrice?: boolean;
  id?: string;
}) {
  return (
    <p id={id} className="text-xs text-gray-300 leading-relaxed" data-testid="renewal-notice">
      <strong className="text-white">
        {name}: {price} per {interval}, renews automatically.
      </strong>{" "}
      Your card is charged {price} today and again every {interval} on the same date until you cancel.
      {locksPrice && " Founding members keep this price while they stay subscribed."} Cancel any time under
      Manage subscription on the Billing page. Cancelling stops future charges and your plan returns to Free
      at the end of the period you paid for. We do not refund the current period unless the law requires it.{" "}
      <Link href="/terms#plans" target="_blank" className="text-brand-blue-400 underline">
        Terms, section 6
      </Link>
      .
    </p>
  );
}
