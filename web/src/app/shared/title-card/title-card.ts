import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { RouterLink } from '@angular/router';
import { TranslocoPipe } from '@jsverse/transloco';

import { Picture } from '../picture/picture';

import { type TitleCard as Card } from '../../core/catalog/catalog.service';

/** One poster tile of a grid. Shows initials on a token surface until images exist. */
@Component({
  selector: 'app-title-card',
  imports: [RouterLink, TranslocoPipe, Picture],
  templateUrl: './title-card.html',
  styleUrl: './title-card.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class TitleCard {
  readonly title = input.required<Card>();

  readonly initials = computed(() =>
    this.title()
      .name.split(/\s+/)
      .filter((w) => w.length > 0)
      .slice(0, 2)
      .map((w) => w[0]?.toUpperCase() ?? '')
      .join(''),
  );
}
