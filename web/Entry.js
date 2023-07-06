import DbTable from './Contestant.js'
import { friendlyProblem, friendlyFix } from './utils.js'
import { ref, computed } from 'vue'

const EventRow = {
  props: {
    event: Object,
    issues: Array,
    relevant: Object,
    partners: Array,
  },
  setup(props) {
    const theseIssues = computed(() => {
      let { issues, event } = props;
      if (!issues || !event) { return []; }

      return issues.filter((issue) => {
          return (
            issue.problem.data && issue.problem.data.event === event.id
              && (
                !issue.problem.data.round
                || issue.problem.data.round === event.round
              )
          )
      })
    })

    const bgColor = computed(() => {
      return theseIssues.value.length ? "bg-red-400" : "#f9f9f9"
    })

    function problemName(issue) {
      return friendlyProblem(issue)
    }

    function suggestedFix(issue) {
      if (issue.fix.name === "UseThisRecord") {
        let other = props.relevant[issue.fix.data];
        if (other) {
          return `Maybe they meant ${other.first_name} ${other.last_name} with ID ${other.igra_number}?`
        }
      }
      return friendlyFix(issue)
    }

    const partnerInfo = computed(() => {
      let { event, partners, relevant } = props
      if (!event.partners || !partners) {
        return []
      }

      let eventPartners = partners.filter((p) => {
        return p.event === event.id && p.round === event.round
      });

      // match the user value to the correct partner.
      return event.partners.map((nameOrNum) => {
        let lowerNN = nameOrNum.trim().toLowerCase()
        if (lowerNN === "") { return {} }

        // among their known partners, find a match
        // if the lowerNN contains the IGRA number or partner name
        return eventPartners.find((p) => {
          if (lowerNN.includes(p.igra_number)) { return true }

          let dbPartner = relevant[p.igra_number]
          if (!dbPartner) { return false }

          let partnerName = `${dbPartner.first_name} ${dbPartner.last_name}`.toLowerCase()
          let partnerLegalName = `${dbPartner.legal_first} ${dbPartner.legal_last}`.toLowerCase()
          return lowerNN.includes(partnerName) || lowerNN.includes(partnerLegalName)
        })
      }).map((p) => { return p ? relevant[p.igra_number] : undefined })
    })

    function formatPartner(p) {
      if (!p) { return }
      return `${p.igra_number} ${p.first_name} ${p.last_name}`
    }

    return {
      problemName,
      suggestedFix,
      theseIssues,
      bgColor,
      partnerInfo,
      formatPartner,
    }
  },
  template: `
      <tr :class="bgColor" class="text-center">
        <td>{{event.id}}</td>
        <td>{{event.round}}</td>
        <td>{{event.partners[0]}}</td>
        <td>{{event.partners[1]}}</td>
        <td>{{event.partners[2]}}</td>
      </tr>
      <tr :class="bgColor" class="text-left text-sm" v-for="issue in theseIssues">
        <td colspan="5" class="">
        → {{problemName(issue)}}
        {{suggestedFix(issue)}}
        </td>
      </tr>
      <tr>
        <td></td>
        <td></td>
        <td v-if="partnerInfo[0]">Match: {{formatPartner(partnerInfo[0])}}</td>
        <td v-else></td>
        <td v-if="partnerInfo[1]">Match: {{formatPartner(partnerInfo[1])}}</td>
        <td v-else></td>
        <td v-if="partnerInfo[2]">Match: {{formatPartner(partnerInfo[2])}}</td>
        <td v-else></td>
      </tr>
      <tr>
        <td></td>
        <td></td>
        <td></td>
        <td></td>
        <td></td>
      </tr>
  `
};


const PersonalDataRow = {
  props: {
    name: String,
    field: String | null,
    fields: Array | null,
    regValue: String,
    dbValue: String,
    issues: Array,
    relevant: Object | null,
  },
  setup(props) {
    function isThisField(issue) {
      let { issues, field, fields } = props;
      if (!issues) { return false; }
      let compareTo = field ? [field] : (fields ?? []);
      return compareTo.some((f) => issue.problem.data.field === f)
    }

    // Did the contestant fill the field?
    const missingRegValue = computed(() => {
      let { issues } = props;
      if (!issues) { return false; }

      return issues.some((issue) => {
          return (issue.problem.name === "NoValue" && isThisField(issue))
      })
    })

    // Do we have a database value, but different from this one?
    const isDbMismatch = computed(() => {
      let { issues, field, fields } = props;
      if (!issues) { return false; }
      let compareTo = field ? [field] : (fields ?? []);

      return issues.some((issue) => {
          return (
            issue.problem.name === "DbMismatch"
            && compareTo.some((f) => issue.problem.data.field === f)
          )
      })
    })

    // Should we expect _any_ DB data?
    const missingDbValue = computed(() => {
      let { issues } = props;
      if (!issues) { return false; }
      return issues.some((issue) => {
        return (
          issue.problem.name === "NotAMember"
          || issue.problem.name === "MaybeAMember"
          || issue.problem.name === "NoPerfectMatch"
        )
      });
    })

    const maybeMatches = computed(() => {
      let { relevant } = props

      return relevant ? relevant : []
    })

    return {
      missingRegValue,
      isDbMismatch,
      missingDbValue,
      maybeMatches,
    }
  },
  template: `
  <tr>
    <td class="text-end font-bold">{{name}}</td>

    <td v-if="missingRegValue" class="bg-red-400">(missing)</td>
    <td v-else :class="isDbMismatch ? 'bg-yellow-400' : ''">
        <slot name="regValue">{{regValue}}</slot>
    </td>

    <td v-if="missingDbValue" class="text-center">-</td>
    <td v-else :class="isDbMismatch ? 'bg-yellow-400' : ''">
        <slot name="dbValue">{{dbValue}}</slot>
    </td>

    <td v-for="maybe in maybeMatches">
      <slot name="maybeMatches" v-bind="maybe"></slot>
    </td>
  </tr>
  `
};

export default {
  components: {
    DbTable,
    EventRow,
    PersonalDataRow,
  },
  props: {
    contestant: Object,
    partners: Array,
    found: Object,
    events: Array,
    issues: Array,
    relevant: Object,
  },
  setup(props) {

    function dbData(key) {
      return props.found ? props.found[key] : "-"
    }

    const dbContestantCat = computed(() => {
      if (!props.found) {
        return "-"
      }
      return props.found.sex === "M" ? "Cowboys" : "Cowgirls"
    })

    const contestantBday = computed(() => {
      let { contestant } = props;
      return `${contestant.dob.month}/${contestant.dob.day}/${contestant.dob.year}`
    })

    function fullName(first, last) {
      if (!first) { return last ?? undefined; }
      if (!last) { return first ?? undefined; }
      return `${first} ${last}`
    }

    return {
      dbData,
      dbContestantCat,
      fullName,
      contestantBday,
    }
  },
  template: `
  <article>
    <div class="grid grid-cols-3">
    <section>
      <header class="text-lg">Registration Data</header>

      <table class="table-auto">
        <thead>
          <tr>
            <th>Property</th>
            <th>Registration</th>
            <th>Database Entry</th>

            <th>Possible Match</th>
          </tr>
        </thead>
        <tbody>

          <personal-data-row
            name="IGRA Number"
            field="IGRANumber"
            :regValue=contestant.association?.igra
            :dbValue=found?.igra_number
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Association"
            field="Association"
            :regValue=contestant.association?.member_assn
            :dbValue=found?.association
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Legal Name"
            field="LegalName"
            :regValue="fullName(contestant.first_name, contestant.last_name)"
            :dbValue="fullName(found?.legal_first, found?.legal_last)"
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Performance Name"
            field="PerformanceName"
            :regValue=contestant.performance_name
            :dbValue="fullName(found?.first_name, found?.last_name)"
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Date of Birth"
            field="DateOfBirth"
            :regValue=contestantBday
            :dbValue=found?.birthdate
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Competes With"
            field="CompetitionCategory"
            :regValue=contestant.gender
            :dbValue=dbContestantCat
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Address"
            :fields='["AddressLine", "City", "Region", "Country", "PostalCode"]'
            :regValue=contestant.gender
            :dbValue=dbContestantCat
            :issues=issues
            :relevant=relevant
            class="oldstyle-nums"
          >
            <template #regValue>
              {{contestant.address.address_line_1}}<br>
              <template v-if="contestant.address.address_line_2">
              {{contestant.address.address_line_2}}<br>
              </template>
              {{contestant.address.city}}, {{contestant.address.region}} {{contestant.address.zip_code}}
              <template v-if='contestant.address.country != "United States"'><br>{{contestant.address.country}}</template>
            </template>

            <template #dbValue>
              {{found?.address}}<br>{{found?.city}}, {{found?.state}} {{found?.zip}}
            </template>

            <template #maybeMatches="maybe">
              {{maybe.address}}<br>{{maybe.city}}, {{maybe.state}} {{maybe.zip}}
            </template>
          </personal-data-row>

          <personal-data-row
            name="Email"
            field="Email"
            :regValue=contestant.address.email
            :dbValue=found?.email
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Cell Phone"
            field="CellPhone"
            :regValue=contestant.cell_phone_no
            :dbValue=found?.cell_phone
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Home Phone"
            field="HomePhone"
            :regValue=contestant.home_phone_no
            :dbValue=found?.home_phone
            :issues=issues
          ></personal-data-row>

          <personal-data-row
            name="Note to Director"
            field="NoteToDirector"
            :regValue=contestant.note_to_director
            :issues=issues
          ></personal-data-row>


        </tbody>
      </table>
    </section>

    <section v-if="found">
      <header class="text-lg">Matching Database Entry</header>
      <db-table :data=found></db-table>
    </section>

    </div>


    <section>
      <header class="text-lg">Registered Events</header>
      <table class="table-auto">
        <thead>
          <tr>
            <th>Event</th>
            <th>Round</th>
            <th>Partner 1</th>
            <th>Partner 2</th>
            <th>Partner 3</th>
          </tr>
        </thead>
        <tbody>
          <event-row v-for="event in events"
            :event=event
            :issues=issues
            :relevant=relevant
            :partners=partners
          ></event-row>
        </tbody>
      </table>
    </section>


    <section v-if="issues">
      <header class="text-lg">Problems</header>
      <table class="table-auto">
        <thead>
          <tr>
            <th>Problem</th>
            <th></th>
            <th>Suggestion</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="issue in issues">
            <td>{{issue.problem.name}}</td>
            <td><template v-if="issue.problem.data">{{issue.problem.data}}</template></td>
            <td>{{issue.fix.name}}</td>
            <td><template v-if="issue.fix.data">{{issue.fix.data}}</template></td>
          </tr>
        </tbody>
      </table>
    </section>

  </article>
  `
}